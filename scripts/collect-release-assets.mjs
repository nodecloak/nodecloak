import fs from 'node:fs';
import path from 'node:path';
import {projectRoot,releaseVersion,releaseFiles} from './release-utils.mjs';
const version=releaseVersion();
const platform=process.argv[2];
const native={'windows-x64':['win32','x64'],'macos-arm64':['darwin','arm64'],'macos-x64':['darwin','x64'],'linux-x64':['linux','x64']}[platform];
if(!native || native[0]!==process.platform || native[1]!==process.arch) throw new Error(`Runner does not match release platform: ${platform}`);
const release=path.join(projectRoot,'src-tauri/target/release');
const files=releaseFiles(platform,version,Boolean(process.env.RELEASE_TAG || (process.platform==='darwin' && process.env.MACOS_NOTARIZE==='true')));
const output=path.join(projectRoot,'release-assets');
fs.mkdirSync(output,{recursive:true});
for(const [relative,name] of files) {
  const source=path.join(release,relative);
  if(!fs.statSync(source).isFile() || fs.statSync(source).size===0) throw new Error(`Missing build package: ${source}`);
  fs.copyFileSync(source,path.join(output,name));
  console.log(`Collected ${name}`);
}
