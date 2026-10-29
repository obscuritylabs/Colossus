'use strict';
const fs = require('node:fs');
const path = require('node:path');
const tls = require('node:tls');
const { X509Certificate } = require('node:crypto');
const [root, rawPort] = process.argv.slice(2);
const port = Number(rawPort);
if (!root || !Number.isInteger(port) || port < 1 || port > 65535) {
  throw new Error('Expected run directory and TCP port.');
}
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'manifest.json'), 'utf8'));
if (manifest.schemaVersion !== 3 || manifest.profile !== 'full-hosted-v1') throw new Error('Complete local hosting profile required.');
const privateRoot = manifest.hosted.privateAllocation.path;
const ca = fs.readFileSync(path.join(privateRoot, 'connector-ca.pem'));
const expected = new X509Certificate(fs.readFileSync(path.join(privateRoot, 'connector-server.pem'))).fingerprint256;
const receipt = { hostname: 'runtimes.colossus.localhost', port, certificateMatchesBackend: false, certificateVerified: false, negotiatedProtocol: null, clientCertificateRequired: false };
const socket = tls.connect({ host: '127.0.0.1', port, servername: receipt.hostname, ALPNProtocols: ['h2'], ca, rejectUnauthorized: true });
const timer = setTimeout(() => socket.destroy(new Error('Connector probe timed out')), 10000);
socket.on('secureConnect', () => {
  receipt.certificateMatchesBackend = socket.getPeerCertificate().fingerprint256 === expected;
  receipt.certificateVerified = socket.authorized;
  receipt.negotiatedProtocol = socket.alpnProtocol;
  if (!receipt.certificateMatchesBackend || !receipt.certificateVerified || receipt.negotiatedProtocol !== 'h2') {
    socket.destroy(new Error('Connector TLS did not reach the expected backend'));
  } else {
    socket.write(Buffer.from('PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n', 'ascii'));
  }
});
socket.on('error', error => {
  if (/certificate required/i.test(error.message)) receipt.clientCertificateRequired = true;
  else receipt.error = error.message;
});
socket.on('data', () => socket.end());
socket.on('close', () => {
  clearTimeout(timer);
  if (!receipt.certificateMatchesBackend || !receipt.certificateVerified || receipt.negotiatedProtocol !== 'h2' || !receipt.clientCertificateRequired || receipt.error) process.exitCode = 1;
  receipt.verifiedAtUtc = new Date().toISOString();
  fs.writeFileSync(path.join(root, 'connector-verification.json'), JSON.stringify(receipt, null, 2) + '\n');
});
