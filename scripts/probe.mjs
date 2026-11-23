import { request } from 'node:http';

const [selected, expected] = process.argv.slice(2);
if (!/^\d+$/.test(selected) || +selected < 1024 || +selected > 65535) {
  throw new Error('Expected a loopback port');
}
const url = 'http://127.0.0.1:' + selected;
const options = () => ({ redirect: 'error', signal: AbortSignal.timeout(2500) });
const get = (path, extra = {}) => fetch(url + path, { ...options(), ...extra });
const probe = { http: false, interface: false, websocket: false };

function protectedWebSocket() {
  return new Promise(resolve => {
    const upgrade = request(url + '/ws/events', {
      signal: AbortSignal.timeout(2500),
      headers: {
        Connection: 'Upgrade',
        Upgrade: 'websocket',
        Origin: url,
        'Sec-WebSocket-Version': '13',
        'Sec-WebSocket-Key': 'dGhlIHNhbXBsZSBub25jZQ==',
      },
    }, response => {
      response.resume();
      resolve(response.statusCode === 403);
    });
    upgrade.on('upgrade', (_response, socket) => {
      socket.destroy();
      resolve(false);
    });
    upgrade.on('error', () => resolve(false));
    upgrade.end();
  });
}
try {
  const health = await get('/api/health');
  const value = await health.json();
  probe.http = health.ok && value.status === 'ok' && value.service === 'cirnet-api' && value.hardware_mode === 'simulation';
  if (probe.http) probe.websocket = await protectedWebSocket();
  const page = await get('/');
  const body = await page.text();
  if (expected === 'vite') {
    probe.interface = page.ok && body.includes('/@vite/client');
  } else {
    const metadata = await get('/interface-artifact.json');
    const data = await metadata.json();
    const entry = /src="(\/assets\/[^"]+\.js)"/.exec(body)?.[1];
    const asset = entry ? await get(entry) : null;
    probe.interface = page.ok && metadata.ok && data.name === 'cirnet-interface-next' && data.kind === 'static' &&
      data.sourceFingerprint === expected && asset?.ok && /javascript/.test(asset.headers.get('content-type'));
    await asset?.body?.cancel();
  }
} catch {
  // Credentials and response bodies stay out of status and logs.
}
probe.healthy = probe.http && probe.interface && probe.websocket;
console.log(JSON.stringify(probe));
