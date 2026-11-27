# Live tests

The regular test suite never leaves the machine: LLM adapters, the MCP client and the chat platforms are tested against mock servers. The live tests check the same code against the real services. They need credentials, some of them cost money, and the chat tests post a message, so they are ignored by default and each one skips itself when its variables are not set.

Run everything you have configured:

```bash
bash scripts/live-tests.sh
```

Set only the variables for what you want to check; the rest is reported as `skipped`.

## LLM providers

Model names are required and never guessed: use one your account can call.

| Check | Variables |
|-------|-----------|
| Anthropic | `ANTHROPIC_API_KEY`, `LIVE_ANTHROPIC_MODEL` |
| Gemini | `GEMINI_API_KEY`, `LIVE_GEMINI_MODEL` |
| OpenAI | `OPENAI_API_KEY`, `LIVE_OPENAI_MODEL` |
| Ollama | `LIVE_OLLAMA_MODEL`, optionally `LIVE_OLLAMA_URL` (default `http://localhost:11434`) |

Each provider gets the same three checks: a plain answer with token usage, a streamed answer that ends with a final chunk carrying usage, and a tool call where the adapter supports tools. Expect a few hundred tokens per provider.

`rejected_key_is_an_authentication_error` only needs `LIVE_ANTHROPIC_MODEL`: it sends an invalid key on purpose.

## MCP servers

| Check | Variables |
|-------|-----------|
| stdio | `LIVE_MCP_COMMAND`, for example `npx -y @modelcontextprotocol/server-everything` |
| HTTP | `LIVE_MCP_URL`, optionally `LIVE_MCP_API_KEY` |

Both connect, list the tools and generate the TypeScript bindings. To call a tool as well, set `LIVE_MCP_TOOL` to its name and `LIVE_MCP_ARGUMENTS` to its arguments as JSON.

The stdio check downloads and runs the server you name: only use one you trust. The server process gets a minimal environment, not your shell's.

## Chat platforms

Each check starts the bot, waits for it to connect and posts one message to the channel you name.

| Check | Variables |
|-------|-----------|
| Discord | `DISCORD_BOT_TOKEN`, `LIVE_DISCORD_CHANNEL_ID` |
| Telegram | `TELEGRAM_BOT_TOKEN`, `LIVE_TELEGRAM_CHAT_ID` |
| Slack | `SLACK_BOT_TOKEN`, `SLACK_APP_TOKEN`, `LIVE_SLACK_CHANNEL_ID` |

## Keeping credentials out of the repository

Export the variables in your shell, or put them in `.env.local` (ignored by git) and load it yourself with `set -a; source .env.local; set +a`. Do not paste keys into commands that end up in shell history on a shared machine.
