// Ask the packaged service to run an isolated fake agent which invokes its CLI.
// Nothing reaches a model provider; Python prepares the fake agent and its home.
import { readFile } from 'node:fs/promises';

const [serviceFile, directory] = process.argv.slice(2);
const { port, token } = JSON.parse(await readFile(serviceFile, 'utf8'));
const socket = new WebSocket(`ws://127.0.0.1:${port}/`);
let next = 0;
let stage = 'connecting';
const waiting = new Map();
let ready;
const connected = new Promise(resolve => ready = resolve);
const timeout = setTimeout(() => { console.error(`Packaged agent check timed out while ${stage}`); process.exit(1); }, 15000);
socket.onopen = () => socket.send(JSON.stringify({ type: 'hello', token }));
socket.onerror = () => { throw new Error('Could not connect to the isolated service'); };
socket.onmessage = ({ data }) => {
  const message = JSON.parse(data);
  if (message.type === 'event' && message.event.type === 'snapshot') ready();
  if (message.type === 'reply') {
    const pending = waiting.get(message.id);
    if (pending) {
      waiting.delete(message.id);
      if (message.outcome.type === 'done') pending.resolve(message.outcome.answer);
      else pending.reject(new Error(message.outcome.message));
    }
  }
};
function call(request) {
  const id = ++next;
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    socket.send(JSON.stringify({ type: 'call', id, request }));
  });
}
try {
  await connected;
  stage = 'adding the project';
  const { project } = await call({ type: 'add_project', name: 'Packaged agent check', directory });
  stage = 'starting the thread';
  const { thread } = await call({ type: 'start_thread', project_id: project.id, agent: 'claude', model: null, title: '' });
  stage = 'sending the message';
  await call({ type: 'send_message', thread_id: thread.id, text: 'Check the bundled command', to: 'author' });
  stage = 'waiting for the agent proof';
  // The fake agent writes this only after its real CLI call succeeds.
  for (;;) {
    try {
      const proof = JSON.parse(await readFile(`${directory}/agent-proof.json`, 'utf8'));
      if (!proof.bundled) throw new Error('The agent did not use the bundled command');
      break;
    } catch (error) {
      if (error.code !== 'ENOENT' && !(error instanceof SyntaxError)) throw error;
      await new Promise(resolve => setTimeout(resolve, 50));
    }
  }
} finally {
  clearTimeout(timeout);
  socket.close();
}
