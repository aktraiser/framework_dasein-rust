//! Durable boundaries for one ChatAgent invocation. The store is scoped to a
//! single invocation and must serialize writers. Uncertain effects fail closed.
use super::{
    async_trait, AgentError, AgentResponse, AgentThread, Arc, ChatAgent, ChatMessage, LLMAdapter,
    LLMMessage,
};
use dasein_agentic_llm::{FinishReason, LLMResponse, ToolCall};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExecutionCheckpoint {
    pub schema_version: u32,
    pub binding: String,
    pub thread: AgentThread,
    pub round: usize,
    pub tokens: u32,
    pub model: String,
    pub pending: Vec<ToolCall>,
    pub next_tool: usize,
    pub in_flight: Option<String>,
    pub terminal: Option<String>,
    pub completed: Option<AgentResponse>,
}

#[async_trait]
pub trait AgentExecutionStore: Send + Sync {
    async fn load(&self) -> Result<Option<AgentExecutionCheckpoint>, AgentError>;
    /// Return only after durable storage. A failure must stop execution.
    async fn save(&self, checkpoint: &AgentExecutionCheckpoint) -> Result<(), AgentError>;
}

impl<L: LLMAdapter + 'static> ChatAgent<L> {
    /// Scope the store to one invocation, including its initial input and phase.
    pub fn with_execution_store(mut self, store: Arc<dyn AgentExecutionStore>) -> Self {
        self.execution_store = Some(store);
        self
    }

    pub(super) async fn run_durable(
        &self,
        messages: Vec<ChatMessage>,
        thread: &mut AgentThread,
        store: &dyn AgentExecutionStore,
    ) -> Result<AgentResponse, AgentError> {
        let all_tools: Vec<dasein_agentic_llm::Tool> = self
            .tools
            .iter()
            .filter(|t| t.enabled)
            .map(|t| {
                Ok(dasein_agentic_llm::Tool {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: serde_json::to_value(&t.parameters)?,
                })
            })
            .collect::<Result<_, AgentError>>()?;
        if !all_tools.is_empty() && !self.llm.supports_tools() {
            return Err(AgentError::tool(
                "configuration",
                "LLM adapter does not support tools",
            ));
        }
        for t in &all_tools {
            if !self.handlers.contains_key(&t.name) {
                return Err(AgentError::tool(&t.name, "No execution handler registered"));
            }
        }
        let binding = serde_json::to_string(
            &serde_json::json!({"agent":self.name,"provider":self.llm.provider(),"model":self.llm.model(),"system":self.system_prompt,"tools":all_tools,"rounds":self.max_tool_rounds,"input":messages.iter().map(|m|serde_json::json!({"role":m.role,"content":m.content,"tool_calls":m.tool_calls,"tool_call_id":m.tool_call_id})).collect::<Vec<_>>()}),
        )?;
        let mut state = if let Some(saved) = store.load().await? {
            if saved.schema_version != 1 || saved.binding != binding {
                return Err(AgentError::tool(
                    "checkpoint",
                    "Checkpoint incompatible with this invocation",
                ));
            }
            saved
        } else {
            thread.add_messages(messages);
            AgentExecutionCheckpoint {
                schema_version: 1,
                binding,
                thread: thread.clone(),
                round: 0,
                tokens: 0,
                model: self.llm.model().into(),
                pending: vec![],
                next_tool: 0,
                in_flight: None,
                terminal: None,
                completed: None,
            }
        };
        *thread = state.thread.clone();
        if let Some(response) = &state.completed {
            return Ok(response.clone());
        }
        if let Some(action) = &state.in_flight {
            return Err(AgentError::tool("checkpoint", format!("Résultat incertain pour {action}. Reprise bloquée pour éviter un double appel ; réconciliation nécessaire.")));
        }
        loop {
            if state.thread.metadata.get("output_limit_exhausted") == Some(&serde_json::json!(true))
            {
                return Err(AgentError::tool("output_limit", "Output limit reached after two bounded recovery attempts; no incomplete tool call executed"));
            }
            if state.round > self.max_tool_rounds {
                return Err(AgentError::tool("limit", "Maximum tool rounds reached"));
            }
            store.save(&state).await?;
            if state.pending.is_empty() {
                let tools: Vec<_> = all_tools
                    .iter()
                    .filter(|t| {
                        self.tool_filter
                            .as_ref()
                            .map(|f| f(&t.name, state.round))
                            .unwrap_or(true)
                    })
                    .cloned()
                    .collect();
                state.in_flight = Some(format!("model:round:{}", state.round));
                store.save(&state).await?;
                let input = self.build_llm_messages(&state.thread);
                let response = if tools.is_empty() {
                    self.llm.generate(&input).await
                } else {
                    self.llm.generate_with_tools(&input, &tools).await
                };
                let response: LLMResponse = match response {
                    Ok(r) => r,
                    Err(e) => {
                        state.in_flight = None;
                        store.save(&state).await?;
                        return Err(e.into());
                    }
                };
                state.in_flight = None;
                state.tokens = state.tokens.saturating_add(response.tokens_used.total);
                state.model = response.model.clone();
                // A truncated response is not a completed reasoning/tool step. Retry
                // at the same boundary, bounded durably so restart cannot reset it.
                if response.finish_reason == FinishReason::Length
                    && (!response.tool_calls.is_empty() || self.completion_check.is_some())
                {
                    let attempts = state
                        .thread
                        .metadata
                        .get("output_limit_retries")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    if attempts >= 2 {
                        state
                            .thread
                            .metadata
                            .insert("output_limit_exhausted".into(), serde_json::json!(true));
                        store.save(&state).await?;
                        return Err(AgentError::tool("output_limit", "Output limit reached after two bounded recovery attempts; no incomplete tool call executed"));
                    }
                    state.thread.metadata.insert(
                        "output_limit_retries".into(),
                        serde_json::json!(attempts + 1),
                    );
                    state.thread.add_response(
                        "Incomplete response discarded: output limit reached.",
                        Some(response.tokens_used.total),
                    );
                    state.thread.add_message(LLMMessage::user(if state.round >= self.max_tool_rounds {
                        "Output truncated. Finish now using only the terminal tool with a short factual summary and limitations. Do not search or repeat previous actions."
                    } else {
                        "Output truncated. Call one tool at a time with short arguments, or finish with a concise answer. Do not repeat previously executed actions."
                    }).into());
                    store.save(&state).await?;
                    continue;
                }
                if response.tool_calls.is_empty() {
                    state
                        .thread
                        .add_response(&response.content, Some(response.tokens_used.total));
                    if let Some(Err(reason)) =
                        self.completion_check.as_ref().map(|f| f(&response.content))
                    {
                        state.thread.add_message(
                            LLMMessage::user(format!("Completion rejected: {reason}")).into(),
                        );
                        state.round += 1;
                        store.save(&state).await?;
                        continue;
                    }
                    let result = AgentResponse::with_messages(
                        response.content,
                        state.thread.messages.clone(),
                    )
                    .with_tokens(state.tokens)
                    .with_model(state.model.clone())
                    .with_truncated(response.finish_reason == FinishReason::Length);
                    state.completed = Some(result.clone());
                    store.save(&state).await?;
                    *thread = state.thread;
                    return Ok(result);
                }
                if state.round == self.max_tool_rounds
                    && !response
                        .tool_calls
                        .iter()
                        .all(|c| self.terminal_tools.contains(&c.name))
                {
                    return Err(AgentError::tool("limit", "Maximum tool rounds reached"));
                }
                let mut ids = std::collections::HashSet::new();
                for c in &response.tool_calls {
                    if !tools.iter().any(|t| t.name == c.name)
                        || !c.arguments.is_object()
                        || c.id.is_empty()
                        || !ids.insert(c.id.clone())
                    {
                        return Err(AgentError::tool(
                            &c.name,
                            "Unregistered tool or invalid call",
                        ));
                    }
                }
                state.thread.add_message(
                    LLMMessage::assistant_with_tool_calls(
                        response.content,
                        response.tool_calls.clone(),
                    )
                    .into(),
                );
                state.pending = response.tool_calls;
                state.next_tool = 0;
                store.save(&state).await?;
            }
            while state.next_tool < state.pending.len() {
                let call = state.pending[state.next_tool].clone();
                let value =
                    if state.terminal.is_some() {
                        serde_json::json!({"skipped":"Invocation already finalized"})
                    } else {
                        state.in_flight = Some(format!("tool:{}:{}", call.name, call.id));
                        store.save(&state).await?;
                        let handler = self.handlers.get(&call.name).ok_or_else(|| {
                            AgentError::tool(&call.name, "Handler unavailable on resume")
                        })?;
                        let value =
                            match tokio::time::timeout(self.tool_timeout, handler(call.arguments))
                                .await
                            {
                                Ok(Ok(v)) => v,
                                Ok(Err(e)) => serde_json::json!({"error":e.to_string()}),
                                // A timed out tool may already have performed its side effect.
                                Err(_) => return Err(AgentError::tool(
                                    "checkpoint",
                                    "Tool timed out; result uncertain, automatic replay blocked",
                                )),
                            };
                        state.in_flight = None;
                        if self.terminal_tools.contains(&call.name) && value.get("error").is_none()
                        {
                            let content = value
                                .get("conclusion")
                                .and_then(|v| v.as_str())
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string());
                            if self
                                .completion_check
                                .as_ref()
                                .map(|f| f(&content).is_ok())
                                .unwrap_or(true)
                            {
                                state.terminal = Some(content);
                            }
                        }
                        value
                    };
                state
                    .thread
                    .add_message(LLMMessage::tool_result(call.id, value.to_string()).into());
                state.next_tool += 1;
                store.save(&state).await?;
            }
            if let Some(content) = state.terminal.take() {
                state.thread.add_response(&content, None);
                let result = AgentResponse::with_messages(content, state.thread.messages.clone())
                    .with_tokens(state.tokens)
                    .with_model(state.model.clone());
                state.completed = Some(result.clone());
                store.save(&state).await?;
                *thread = state.thread;
                return Ok(result);
            }
            state.pending.clear();
            state.next_tool = 0;
            state.round += 1;
        }
    }
}
