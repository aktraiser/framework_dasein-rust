  #!/usr/bin/env python3                                                                                                    
  import http.server                                                                                                        
  import json                                                                                                               
  import subprocess                                                                                                         
  import socketserver                                                                                                       
                                                                                                                            
  PORT = 8080                                                                                                               
                                                                                                                            
  class Handler(http.server.BaseHTTPRequestHandler):                                                                        
      def do_GET(self):                                                                                                     
          if self.path == '/health':                                                                                        
              self.send_response(200)                                                                                       
              self.send_header('Content-Type', 'application/json')                                                          
              self.end_headers()                                                                                            
              self.wfile.write(json.dumps({"status": "healthy"}).encode())                                                  
                                                                                                                            
      def do_POST(self):                                                                                                    
          if self.path == '/execute':                                                                                       
              length = int(self.headers.get('Content-Length', 0))                                                           
              body = json.loads(self.rfile.read(length))                                                                    
              code = body.get('code', '')                                                                                   
              timeout = body.get('timeout_ms', 5000) / 1000                                                                 
              try:                                                                                                          
                  result = subprocess.run(['sh', '-c', code], capture_output=True, text=True, timeout=timeout)              
                  response = {"success": True, "exit_code": result.returncode, "stdout": result.stdout, "stderr":           
  result.stderr}                                                                                                            
              except subprocess.TimeoutExpired:                                                                             
                  response = {"success": False, "error": "timeout"}                                                         
              except Exception as e:                                                                                        
                  response = {"success": False, "error": str(e)}                                                            
              self.send_response(200)                                                                                       
              self.send_header('Content-Type', 'application/json')                                                          
              self.end_headers()                                                                                            
              self.wfile.write(json.dumps(response).encode())                                                               
                                                                                                                            
  with socketserver.TCPServer(("0.0.0.0", PORT), Handler) as httpd:                                                         
      print(f"Agent listening on port {PORT}")                                                                              
      httpd.serve_forever()  