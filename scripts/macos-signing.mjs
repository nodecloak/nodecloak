import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {projectRoot, releaseVersion} from './release-utils.mjs';

const stateFile = path.join(process.env.RUNNER_TEMP || os.tmpdir(), 'nodecloak-macos-signing.json');
function run(command, args, options = {}) {
  const result = spawnSync(command, args, {cwd: projectRoot, encoding: 'utf8', ...options});
  if (result.error) throw new Error(`Unable to run ${command}`);
  if (result.status !== 0) throw new Error(`${command} failed (${result.status}): ${result.stderr || result.stdout || ''}`);
  return result;
}
function requireSecrets(names) {
  for (const name of names) if (!process.env[name]) throw new Error(`Missing Actions secret: ${name}`);
}
function readState() { return JSON.parse(fs.readFileSync(stateFile, 'utf8')); }

function cleanup() {
  if (!fs.existsSync(stateFile)) return;
  const state = readState();
  let failure;
  try { run('security', ['list-keychains', '-d', 'user', '-s', ...state.originalKeychains]); }
  catch (error) { failure = error; }
  try { run('security', ['delete-keychain', state.keychain]); }
  catch (error) { failure ||= error; }
  fs.rmSync(state.directory, {recursive: true, force: true});
  fs.rmSync(stateFile, {force: true});
  if (failure) throw failure;
  console.log('Temporary signing keychain removed.');
}

function prepare() {
  requireSecrets(['APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID']);
  if (!process.env.GITHUB_ENV) throw new Error('Certificate import is only supported on a GitHub runner');
  if (fs.existsSync(stateFile)) throw new Error('A signing keychain already exists; clean it before importing another');
  const originalKeychains = [...run('security', ['list-keychains', '-d', 'user']).stdout.matchAll(/"([^"]+)"/g)].map(match => match[1]);
  const directory = fs.mkdtempSync(path.join(process.env.RUNNER_TEMP || os.tmpdir(), 'nodecloak-signing-'));
  fs.chmodSync(directory, 0o700);
  const keychain = path.join(directory, 'build.keychain-db');
  fs.writeFileSync(stateFile, JSON.stringify({directory, keychain, originalKeychains}), {mode: 0o600});
  const certificate = path.join(directory, 'certificate.p12');
  const password = crypto.randomBytes(32).toString('base64');
  try {
    fs.writeFileSync(certificate, Buffer.from(process.env.APPLE_CERTIFICATE, 'base64'), {mode: 0o600});
    run('security', ['create-keychain', '-p', password, keychain]);
    run('security', ['set-keychain-settings', '-lut', '7200', keychain]);
    run('security', ['unlock-keychain', '-p', password, keychain]);
    run('security', ['import', certificate, '-k', keychain, '-P', process.env.APPLE_CERTIFICATE_PASSWORD, '-T', '/usr/bin/codesign']);
    fs.rmSync(certificate);
    run('security', ['set-key-partition-list', '-S', 'apple-tool:,apple:,codesign:', '-s', '-k', password, keychain]);
    const identities = [...run('security', ['find-identity', '-v', '-p', 'codesigning', keychain]).stdout.matchAll(/\b([A-F0-9]{40})\s+"([^"]+)"/g)];
    if (identities.length !== 1 || !identities[0][2].startsWith('Developer ID Application: ') || !identities[0][2].endsWith(`(${process.env.APPLE_TEAM_ID})`)) {
      throw new Error('PKCS#12 must contain exactly one valid Developer ID Application identity for the configured team');
    }
    run('security', ['list-keychains', '-d', 'user', '-s', keychain, ...originalKeychains]);
    run('xcrun', ['notarytool', 'store-credentials', 'nodecloak-ci', '--apple-id', process.env.APPLE_ID, '--password', process.env.APPLE_PASSWORD, '--team-id', process.env.APPLE_TEAM_ID, '--keychain', keychain]);
    fs.appendFileSync(process.env.GITHUB_ENV, `APPLE_SIGNING_IDENTITY=${identities[0][1]}\n`);
    console.log(`Imported ${identities[0][2]} and validated notarization credentials.`);
  } catch (error) {
    cleanup();
    throw error;
  }
}

function finish() {
  requireSecrets(['TAURI_SIGNING_PRIVATE_KEY', 'APPLE_TEAM_ID']);
  const version = releaseVersion();
  const state = readState();
  const bundle = path.join(projectRoot, 'src-tauri/target/release/bundle');
  const app = path.join(bundle, 'macos/NodeCloak.app');
  const dmg = path.join(bundle, `dmg/NodeCloak_${version}_${process.arch === 'arm64' ? 'aarch64' : 'x64'}.dmg`);
  const archive = `${app}.tar.gz`;
  for (const file of [app, dmg]) {
    run('codesign', ['--verify', '--deep', '--strict', file]);
    const identity = run('codesign', ['--display', '--verbose=4', file]).stderr;
    if (!identity.includes('Authority=Developer ID Application: ') || !identity.includes(`TeamIdentifier=${process.env.APPLE_TEAM_ID}`)) {
      throw new Error('Package was not signed with the configured Developer ID team');
    }
  }
  run('xcrun', ['stapler', 'validate', app]);
  run('spctl', ['--assess', '--type', 'execute', '--verbose=2', app]);
  const receipt = JSON.parse(run('xcrun', ['notarytool', 'submit', dmg, '--keychain-profile', 'nodecloak-ci', '--keychain', state.keychain, '--wait', '--timeout', '45m', '--output-format', 'json']).stdout);
  if (receipt.status !== 'Accepted') throw new Error(`DMG notarization was not accepted: ${receipt.status}; submission ${receipt.id}`);
  run('xcrun', ['stapler', 'staple', dmg]);
  run('xcrun', ['stapler', 'validate', dmg]);
  run('spctl', ['--assess', '--type', 'open', '--context', 'context:primary-signature', '--verbose=2', dmg]);
  // Generate the updater only after all mutations of the notarized application.
  run('tar', ['-czf', archive, '-C', path.dirname(app), path.basename(app)], {env: {...process.env, COPYFILE_DISABLE: '1'}});
  run(process.execPath, ['scripts/tauri.mjs', 'signer', 'sign', '--app-version', version, archive], {stdio: 'inherit'});
  const extracted = path.join(state.directory, 'updater-check');
  fs.mkdirSync(extracted);
  run('tar', ['-xzf', archive, '-C', extracted]);
  run('codesign', ['--verify', '--deep', '--strict', path.join(extracted, 'NodeCloak.app')]);
  run('xcrun', ['stapler', 'validate', path.join(extracted, 'NodeCloak.app')]);
  // Keep verification receipts separate from the fixed set of public release assets.
  const receipts = path.join(projectRoot, 'notarization-receipts');
  fs.mkdirSync(receipts, {recursive: true});
  fs.writeFileSync(path.join(receipts, `macos-${process.arch}.json`), JSON.stringify({version, architecture: process.arch, dmg: receipt}, null, 2));
  console.log(`Verified signed and notarized ${process.arch} application, DMG and updater archive.`);
}

if (process.platform !== 'darwin') throw new Error('macOS signing requires a macOS runner');
const command = process.argv[2];
if (command === 'prepare') prepare();
else if (command === 'finish') finish();
else if (command === 'cleanup') cleanup();
else throw new Error('Usage: node scripts/macos-signing.mjs prepare|finish|cleanup');
