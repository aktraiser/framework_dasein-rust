#!/usr/bin/env python3
import http.server
import json
import subprocess
import socketserver
import time
import base64
import os

PORT = 8080

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        print(f"{self.address_string()} - {args[0]}")

    def do_GET(self):
        if self.path == '/health':
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.end_headers()
            response = {
                "status": "healthy",
                "firecracker_available": True,
                "kvm_available": True,
                "version": "0.2.0"
            }
            self.wfile.write(json.dumps(response).encode())

    def do_POST(self):
        if self.path == '/execute':
            try:
                length = int(self.headers.get('Content-Length', 0))
                body = json.loads(self.rfile.read(length))

                # Support both plain code and base64-encoded code
                code = body.get('code', '')
                if body.get('base64', False):
                    code = base64.b64decode(code).decode('utf-8')

                timeout = body.get('timeout_ms', 30000) / 1000
                workdir = body.get('workdir', '/tmp')

                # Write code to a temporary script file to avoid shell escaping issues
                script_path = f"/tmp/agent_script_{os.getpid()}.sh"
                with open(script_path, 'w') as f:
                    f.write(code)
                os.chmod(script_path, 0o755)

                start = time.time()
                try:
                    result = subprocess.run(
                        ['/bin/bash', script_path],
                        capture_output=True,
                        text=True,
                        timeout=timeout,
                        cwd=workdir
                    )
                    elapsed = int((time.time() - start) * 1000)
                    response = {
                        "success": True,
                        "exit_code": result.returncode,
                        "stdout": result.stdout,
                        "stderr": result.stderr,
                        "execution_time_ms": elapsed,
                        "artifacts": []
                    }
                except subprocess.TimeoutExpired:
                    response = {"success": False, "error": "timeout"}
                except Exception as e:
                    response = {"success": False, "error": str(e)}
                finally:
                    # Clean up script file
                    try:
                        os.remove(script_path)
                    except:
                        pass

                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(json.dumps(response).encode())
            except Exception as e:
                self.send_response(500)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(json.dumps({"success": False, "error": str(e)}).encode())

with socketserver.TCPServer(("0.0.0.0", PORT), Handler) as httpd:
    print(f"Agent v0.2.0 listening on port {PORT}")
    httpd.serve_forever()
