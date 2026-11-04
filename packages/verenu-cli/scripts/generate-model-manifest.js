import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');
export async function generate() {
  const llm = await readFile(resolve(root, 'src-tauri/src/local_llm/model.rs'), 'utf8');
  const stt = await readFile(resolve(root, 'src-tauri/src/local_stt/model.rs'), 'utf8');
  const fluid = JSON.parse(await readFile(resolve(root, 'src-tauri/src/local_stt/fluid_assets.json'), 'utf8'));
  const files = [];
  const speech = [];
  for (const match of llm.matchAll(/LocalLlmArtifact\s*\{\s*filename:\s*"([^"]+)",\s*sha256:\s*Some\("([a-f0-9]{64})"\)/g)) files.push({ name: match[1], sha256: match[2] });
  for (const match of stt.matchAll(/LocalSttModelManifest\s*\{([^{}]+)\}/g)) {
    const block = match[1];
    const name = block.match(/filename:\s*"([^"]+)"/), sha = block.match(/sha256:\s*Some\("([a-f0-9]{64})"\)/);
    const id = block.match(/\bid:\s*"([^"]+)"/), install = block.match(/install_name:\s*"([^"]+)"/);
    const engine = block.match(/engine_type:\s*LocalSttEngineType::(\w+)/);
    if (id && install) speech.push({ id: id[1], install: install[1], engine: engine?.[1] || null, archive: name?.[1] || null, sha256: sha?.[1] || null, directory: /is_directory:\s*true/.test(block) });
    if (name && sha && /is_directory:\s*false/.test(block)) files.push({ name: name[1], sha256: sha[1] });
  }
  if (files.length < 10) throw new Error('Model manifest extraction incomplete');
  return JSON.stringify({ files, speech, bundles: fluid.map(({id, revision, files}) => ({id, revision, files})) }, null, 2) + '\n';
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await writeFile(resolve(root, 'packages/verenu-cli/src/model-manifest.json'), await generate());
