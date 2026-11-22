import { stat } from 'node:fs/promises';
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const [root, selected, stop] = process.argv.slice(2);
const outcome = join(dirname(stop), 'vite.result');
const { createServer } = await import(pathToFileURL(join(root, 'node_modules/vite/dist/node/index.js')));
const server = await createServer({ root, server: { host: '127.0.0.1', port: Number(selected), strictPort: true } });
let stopRequested = false;
try {
  await server.listen();
  console.log('CIRNET_VITE_READY=http://127.0.0.1:' + selected);
  for (;;) {
    try {
      const request = await stat(stop);
      if (request.isFile()) {
        stopRequested = true;
        break;
      }
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
    await new Promise(resolve => setTimeout(resolve, 200));
  }
} finally {
  // The dev server owns no lab data. Bound local Vite cleanup independently of
  // the native launcher's safety-critical Remote and Hardware shutdown.
  const deadline = setTimeout(() => {
    writeFileSync(outcome, 'timed out\n');
    process.exit(1);
  }, 10000);
  try {
    await server.close();
    writeFileSync(outcome, 'clean\n');
    if (stopRequested) process.exit(0);
  } catch (error) {
    writeFileSync(outcome, 'failed\n');
    console.error(error);
    process.exit(1);
  } finally {
    clearTimeout(deadline);
  }
}
