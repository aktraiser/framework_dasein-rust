#!/usr/local/bin/python3
"""Trusted microVM broker: one bounded request over vsock, never an HTTP host shell."""
import json
import os
import resource
import selectors
import signal
import socket
import struct
import subprocess
import time

MAX_REQUEST = 2 * 1024 * 1024
MAX_OUTPUT = 1024 * 1024


def read_exact(connection, count):
    data = bytearray()
    while len(data) < count:
        chunk = connection.recv(count - len(data))
        if not chunk:
            raise ValueError("truncated request")
        data.extend(chunk)
    return bytes(data)


def child_limits():
    os.setgroups([])
    os.setgid(65534)
    os.setuid(65534)
    resource.setrlimit(resource.RLIMIT_NPROC, (64, 64))
    resource.setrlimit(resource.RLIMIT_NOFILE, (64, 64))
    resource.setrlimit(resource.RLIMIT_FSIZE, (16 * 1024 * 1024,) * 2)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    os.umask(0o077)


def execute(request):
    if request.get("protocol") != 1 or not isinstance(request.get("request_id"), str):
        raise ValueError("invalid protocol")
    code = request.get("code")
    timeout = request.get("timeout_ms")
    maximum = request.get("max_output_bytes")
    if not isinstance(code, str) or len(code.encode()) > 256 * 1024:
        raise ValueError("invalid code size")
    if type(timeout) is not int or not 0 < timeout <= 120000:
        raise ValueError("invalid timeout")
    if type(maximum) is not int or not 0 < maximum <= MAX_OUTPUT:
        raise ValueError("invalid output limit")
    response = dict(protocol=1, request_id=request["request_id"], exit_code=-1,
                    stdout="", stderr="", error=None)
    process = subprocess.Popen(["/bin/sh", "-c", code], stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               cwd="/workspace", start_new_session=True,
                               preexec_fn=child_limits,
                               env={"PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/workspace",
                                    "TMPDIR": "/tmp", "PYTHONDONTWRITEBYTECODE": "1"})
    streams = selectors.DefaultSelector()
    outputs = {"stdout": bytearray(), "stderr": bytearray()}
    streams.register(process.stdout, selectors.EVENT_READ, "stdout")
    streams.register(process.stderr, selectors.EVENT_READ, "stderr")
    deadline = time.monotonic() + timeout / 1000
    try:
        while streams.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                response["error"] = "timeout"
                break
            for key, _ in streams.select(min(remaining, .05)):
                chunk = os.read(key.fileobj.fileno(), 4096)
                if not chunk:
                    streams.unregister(key.fileobj)
                    continue
                if sum(len(value) for value in outputs.values()) + len(chunk) > maximum:
                    response["error"] = "output limit exceeded"
                    break
                outputs[key.data].extend(chunk)
            if response["error"]:
                break
        if response["error"] is None:
            try:
                response["exit_code"] = process.wait(timeout=max(.001, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                response["error"] = "timeout"
    finally:
        # The host also destroys the entire microVM; detached processes cannot
        # survive the host's VM deadline even if they escape this process group.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        streams.close()
        process.stdout.close()
        process.stderr.close()
    for stream, value in outputs.items():
        response[stream] = value.decode("utf-8", errors="replace")
    # UTF-8 replacement can expand the byte count: enforce the wire result cap.
    if len(response["stdout"].encode()) + len(response["stderr"].encode()) > maximum:
        response.update(stdout="", stderr="", error="output limit exceeded")
    return response


def main():
    # PID 1 and the immutable image marker prevent accidental use on a host.
    if os.getpid() != 1 or not os.path.exists("/etc/dasein-firecracker-guest"):
        raise SystemExit("This broker must run as PID 1 inside the prepared microVM image")
    listener = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
    listener.bind((socket.VMADDR_CID_ANY, 5000))
    listener.listen(1)
    while True:
        connection, _ = listener.accept()
        executed = False
        with connection:
            connection.settimeout(125)
            try:
                size = struct.unpack("!I", read_exact(connection, 4))[0]
                if size > MAX_REQUEST:
                    raise ValueError("request too large")
                request = json.loads(read_exact(connection, size))
                executed = True
                response = execute(request)
                data = json.dumps(response, ensure_ascii=False).encode()
                connection.sendall(struct.pack("!I", len(data)) + data)
            except (ValueError, OSError, TypeError):
                # Malformed requests are closed, never interpreted as host code.
                pass
        if not executed:
            continue
        # A microVM handles only one request; the host destroys it after reply.
        listener.close()
        while True:
            time.sleep(3600)


if __name__ == "__main__":
    main()
