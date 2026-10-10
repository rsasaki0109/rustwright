from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import sys
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == '/denied':
            status, title = 403, 'fixture denial'
        else:
            status, title = 200, 'fixture observation'
        body = f'''<!doctype html><html><head><title>{title}</title></head><body>{title}
<script>console.log('compat-fixture-console');setTimeout(()=>{{throw new Error('compat-fixture-js-error')}}, 50);</script></body></html>'''.encode()
        self.send_response(status)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
Path(sys.argv[1]).write_text(str(server.server_port))
server.serve_forever()
