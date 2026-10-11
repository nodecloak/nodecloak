import {spawnSync} from 'node:child_process';
import {projectRoot,releaseVersion} from './release-utils.mjs';
releaseVersion();
const notarizedMac=process.platform==='darwin' && process.env.MACOS_NOTARIZE==='true';
const signed=Boolean(process.env.RELEASE_TAG) || notarizedMac;
if(signed && !process.env.TAURI_SIGNING_PRIVATE_KEY) throw new Error('Signed releases require the repository TAURI_SIGNING_PRIVATE_KEY secret');
const args=['scripts/tauri.mjs','build','--bundles',process.platform==='win32'?'nsis':'app,dmg'];
// The notarized Mac updater is created from the final stapled application.
if(!signed || notarizedMac) args.push('--config','src-tauri/tauri.ci.json');
const result=spawnSync(process.execPath,args,{cwd:projectRoot,stdio:'inherit'});
if(result.error) throw result.error;
process.exitCode=result.status ?? 1;
