    FIRECRACKER_URL=http://65.108.230.227:8080 GEMINI_API_KEY=REDACTED_API_KEY CONTEXT7_API_KEY=REDACTED_CONTEXT7_KEY FAST_MODEL=gemini-2.0-flash SMART_MODEL=claude-haiku-4-5-20251001 cargo run --example multi_executor   --features remote 


  lance le NAT :  nats-server -js 

   API_KEY="MqFDPc59jf5Ua?" ./target/release/firecracker-server    

     curl -X POST http://172.16.0.2:8080/execute -H "Content-Type: application/json" -d '{"code":"rustc --version","timeout_ms":5000}' 

       curl -X POST http://172.16.0.2:8080/execute -H "Content-Type: application/json" -d '{"code":"node --version","timeout_ms":5000}' 

