"""Synthetic real HTTP transfers through the closed, authenticated native bridge."""
import base64
import hashlib
import time
from email import policy
from email.parser import BytesParser

from run_probe import ProbeError

UPLOAD = b"Colossus ordinary owned upload\n" * 3073
DOWNLOAD = bytes(range(256)) * 513
MAX_CHUNK = 64 * 1024


def fixture(page):
    return page.replace(b"</body>", b'''<form id="owned-upload" action="/upload" method="post" enctype="multipart/form-data">
      <label>Owned file upload<input type="file" name="owned_file" aria-label="Owned native file input"></label>
      <button type="submit">Submit owned upload</button></form>
      <a href="/download-redirect">Owned binary download</a>
      <a href="/download-empty">Owned empty download</a>
      <a href="http://denied.invalid/file">Forbidden binary download</a>
      <output id="owned-upload-result">Owned upload waiting</output>
      <script>document.querySelector('#owned-upload').addEventListener('submit', async event => {
        event.preventDefault();
        const response = await fetch('/upload', {method:'POST', body:new FormData(event.target)});
        document.querySelector('#owned-upload-result').textContent = response.ok ? 'Owned upload received' : 'Owned upload rejected';
      });</script></body>''')


def serve_get(handler, origin):
    if handler.path == origin + "/download-redirect":
        handler.send_response(302)
        handler.send_header("Location", origin + "/download-final")
        handler.send_header("Content-Length", "0")
        handler.end_headers()
        return True
    if handler.path in (origin + "/download-final", origin + "/download-empty"):
        content = DOWNLOAD if handler.path.endswith("/download-final") else b""
        handler.send_response(200)
        handler.send_header("Content-Type", "application/octet-stream")
        handler.send_header("Content-Disposition", 'attachment; filename="../../untrusted-name.bin"')
        handler.send_header("Content-Length", str(len(content)))
        handler.end_headers()
        handler.wfile.write(content)
        return True
    return False


def serve_upload(handler, page, receipts):
    length = handler.headers.get("Content-Length", "")
    if not length.isdecimal() or not 0 < int(length) <= 4 * 1024 * 1024 + 64 * 1024:
        raise ProbeError("native multipart upload exceeded its bounded fixture")
    body = handler.rfile.read(int(length))
    message = BytesParser(policy=policy.default).parsebytes(
        ("Content-Type: " + handler.headers.get("Content-Type", "") + "\r\n\r\n").encode() + body)
    parts = list(message.iter_parts())
    if len(parts) != 1 or parts[0].get_param("name", header="content-disposition") != "owned_file":
        raise ProbeError("native upload changed its ordinary form field")
    if parts[0].get_filename() != "upload.txt" or parts[0].get_payload(decode=True) != UPLOAD:
        raise ProbeError("native browser did not send the exact reviewed staged upload")
    receipts.append(True)
    handler.send_response(200)
    handler.send_header("Content-Type", "text/html; charset=utf-8")
    handler.send_header("Content-Length", str(len(page)))
    handler.end_headers()
    handler.wfile.write(page)


def transfer_fixture(channel, command, action, snapshot, submitted, origin):
    def node(name):
        observed = snapshot()
        return next((row for row in observed["observation"]["snapshot"]["nodes"]
                     if row.get("element") and name in row.get("name", "")), None)

    source = node("Owned native file input")
    if source is None:
        raise ProbeError("native snapshot did not issue an ordinary file-input reference")
    artifact = "artifact-" + hashlib.sha256(UPLOAD).hexdigest()
    upload = command({"kind": "upload", "element": source["element"], "artifact_id": artifact, "max_bytes": 4 * 1024 * 1024})
    result = channel.request({"operation": "begin_upload", "request": {"command": upload,
        "descriptor": {"artifact_id": artifact, "size_bytes": len(UPLOAD), "sha256": hashlib.sha256(UPLOAD).hexdigest(), "file_name": "upload.txt"}}})
    receipt = result.get("receipt", {})
    token = receipt.get("transfer_id")
    if result.get("result") != "upload_prepared" or receipt.get("next_offset") != 0 or not isinstance(token, str) or len(token) != 32 or any(c not in "0123456789abcdef" for c in token):
        code = result.get("code")
        if code in ("denied", "stale", "unavailable", "unsupported", "failed", "outcome_unknown", "limit_exceeded", "timed_out"):
            raise ProbeError("native upload preparation rejected: " + code)
        raise ProbeError("native upload preparation did not bind a fresh input")
    request = {key: upload[key] for key in ("binding", "run_id", "session_id", "target", "control_generation")}
    request.update(transfer_id=receipt.get("transfer_id"), offset=0)
    for offset in range(0, len(UPLOAD), MAX_CHUNK):
        request["offset"] = offset
        chunk = UPLOAD[offset:offset + MAX_CHUNK]
        progress = channel.request({"operation": "write_upload", "request": {"transfer": request, "data_base64": base64.b64encode(chunk).decode()}})
        if progress != {"result": "upload_progress", "receipt": {"transfer_id": request["transfer_id"], "next_offset": offset + len(chunk)}}:
            raise ProbeError("native upload changed its ordered bounded byte custody")
    request["offset"] = len(UPLOAD)
    committed = channel.request({"operation": "commit_upload", "request": request})
    if committed.get("result") != "uploaded" or committed.get("observation", {}).get("tab", {}).get("document_id") != upload["target"]["document_id"]:
        raise ProbeError("native upload did not acknowledge its fixed DOM file-input effect")
    submit = node("Submit owned upload")
    if submit is None or action({"kind": "click", "element": submit["element"]}).get("result") != "observed":
        raise ProbeError("native form submit failed")
    deadline = time.monotonic() + 5
    while True:
        observed = snapshot()
        if submitted and any("Owned upload received" in row.get("name", "") for row in observed["observation"]["snapshot"]["nodes"]):
            break
        if time.monotonic() >= deadline:
            raise ProbeError("native browser never acknowledged its actual uploaded bytes over HTTP")
        time.sleep(0.025)

    for name, content in (("Owned binary download", DOWNLOAD), ("Owned empty download", b"")):
        link = node(name)
        if link is None:
            raise ProbeError("native snapshot did not issue the reviewed download link")
        download = command({"kind": "download", "element": link["element"], "max_bytes": 4 * 1024 * 1024})
        received = channel.request({"operation": "download", "command": download})
        descriptor = received.get("descriptor", {})
        if received.get("result") != "downloaded" or descriptor.get("size_bytes") != len(content) or descriptor.get("sha256") != hashlib.sha256(content).hexdigest() or descriptor.get("origin") != origin or any(descriptor.get(key) != download[key] for key in ("session_id", "target", "control_generation")):
            raise ProbeError("native download did not prove its actual bounded file bytes and origin")
        read = {key: download[key] for key in ("binding", "run_id", "session_id", "target", "control_generation")}
        read.update(transfer_id=descriptor.get("transfer_id"), offset=0)
        data = bytearray()
        while True:
            read["offset"] = len(data)
            result = channel.request({"operation": "read_download", "request": read})
            value = result.get("chunk", {})
            if result.get("result") != "download_chunk" or value.get("offset") != len(data):
                raise ProbeError("native download broke its exact ordered transfer")
            encoded = value.get("data_base64")
            if not isinstance(encoded, str) or len(encoded) > ((MAX_CHUNK + 2) // 3) * 4:
                raise ProbeError("native download chunk exceeded the private frame budget")
            chunk = base64.b64decode(encoded, validate=True)
            if len(chunk) > MAX_CHUNK or len(data) + len(chunk) > len(content) or base64.b64encode(chunk).decode() != encoded:
                raise ProbeError("native download returned malformed bounded bytes")
            data.extend(chunk)
            if len(data) == len(content):
                break
            if not chunk:
                raise ProbeError("native download truncated its complete file")
        if data != content:
            raise ProbeError("native downloader returned different actual HTTP bytes")
        read["offset"] = 0
        if channel.request({"operation": "read_download", "request": read}) != {"result": "rejected", "code": "stale"}:
            raise ProbeError("native download remained readable after complete consumption")
    forbidden = node("Forbidden binary download")
    if channel.request({"operation": "download", "command": command({"kind": "download", "element": forbidden["element"], "max_bytes": 4 * 1024 * 1024})}) != {"result": "rejected", "code": "denied"}:
        raise ProbeError("native download accepted a link outside immutable origin admission")
    return {"native_upload_actual_http_bytes": True, "native_download_actual_http_bytes": True,
        "download_allowed_redirect": True, "download_empty": True, "download_origin_denied": True,
        "transfer_ordered_bounded_chunks": True, "download_consumed_revoked": True,
        "page_download_filename_ignored": True}
