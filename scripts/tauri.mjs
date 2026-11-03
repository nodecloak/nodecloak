import { existsSync } from 'node:fs';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import {prepare} from './prepare-singbox.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const env = { ...process.env };
if(['dev','build','bundle'].includes(process.argv[2]))await prepare();
const cargo = path.join(root, '.tools', 'cargo');
const rustup = path.join(root, '.tools', 'rustup');
if (existsSync(path.join(cargo, 'bin', process.platform === 'win32' ? 'cargo.exe' : 'cargo'))) {
  env.CARGO_HOME = cargo;
  env.RUSTUP_HOME = rustup;
  env.PATH = `${path.join(cargo, 'bin')}${path.delimiter}${process.env.PATH || ''}`;
}
const child = spawn(process.execPath, [path.join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), ...process.argv.slice(2)], { cwd: root, env, stdio: 'inherit' });
child.on('error', error => { console.error(error.message); process.exitCode = 1; });
child.on('exit', code => { process.exitCode = code ?? 1; });
