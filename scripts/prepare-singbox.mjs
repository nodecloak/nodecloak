import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {gzipSync} from 'node:zlib';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
export const version='1.14.4';
const root=path.resolve(fileURLToPath(new URL('../',import.meta.url)));
export const platforms={
 'win32-x64':['windows-amd64','zip','99185d75d314a07dbba4a130c8ad986cbeb629a07927975636b341ca61f991df'],
 'darwin-x64':['darwin-amd64-legacy-macos-10.13','tar.gz','4591d9641e4cfda5de331414e8447aac56d10e43e01e413ef1cb8cf0e711a460'],
 'darwin-arm64':['darwin-arm64','tar.gz','0d8c7f5ed5d587b0f1a5e9772a708ec3bfa4ac4c9329604b01d8d9c2ba406838'],
 'linux-x64':['linux-amd64','tar.gz','699a1b510aa4396b2176d32d1181920399ebf1e267835da6f3b598d0f7c24968'],
};
const hash=bytes=>crypto.createHash('sha256').update(bytes).digest('hex');
export async function prepare(platform=`${process.platform}-${process.arch}`) {
 const entry=platforms[platform];if(!entry)throw new Error('Unsupported sing-box build platform: '+platform);
 const [target,extension,digest]=entry;
 const folder=path.join(root,'.tools/singbox',platform);fs.mkdirSync(folder,{recursive:true});
 const name=`sing-box-${version}-${target}.${extension}`;
 const archive=path.join(folder,name);
 if(!fs.existsSync(archive)||hash(fs.readFileSync(archive))!==digest){
  const response=await fetch(`https://github.com/SagerNet/sing-box/releases/download/v${version}/${name}`,{signal:AbortSignal.timeout(180000)});
  if(!response.ok)throw new Error('sing-box download failed: '+response.status);
  const bytes=Buffer.from(await response.arrayBuffer());if(hash(bytes)!==digest)throw new Error('sing-box archive digest mismatch');
  fs.writeFileSync(archive,bytes);
 }
 const binary=`sing-box-${version}-${target}/sing-box${platform.startsWith('win32')?'.exe':''}`;
 let bytes=execFileSync('tar',['-xOf',archive,binary],{maxBuffer:100*1024*1024});
 if(!bytes.length)throw new Error('Empty sing-box executable');
 const nameOnDisk=path.join(folder,platform.startsWith('win32')?'sing-box.exe':'sing-box');
 fs.writeFileSync(nameOnDisk,bytes,{mode:0o700});if(process.platform!=='win32')fs.chmodSync(nameOnDisk,0o700);
 if(platform===`darwin-${process.arch}`&&process.env.APPLE_SIGNING_IDENTITY){
  execFileSync('codesign',['--force','--options','runtime','--timestamp','--sign',process.env.APPLE_SIGNING_IDENTITY,nameOnDisk],{stdio:'inherit'});
  execFileSync('codesign',['--verify','--strict',nameOnDisk]);bytes=fs.readFileSync(nameOnDisk);
 }
 fs.writeFileSync(path.join(folder,'core.gz'),gzipSync(bytes));
 fs.writeFileSync(path.join(folder,'core.sha256'),hash(bytes)+'\n');
 if(platform===`${process.platform}-${process.arch}`){
  const bundle=path.join(root,'.tools/singbox/bundle');fs.mkdirSync(bundle,{recursive:true});fs.writeFileSync(path.join(bundle,'sing-box'),bytes,{mode:0o700});
 }
 const source=path.join(root,'.tools/singbox/source/sing-box-1.14.4-source.tar.gz');
 const sourceSha='0958ccd23b1ea9a141d715d15478b1e040fd04bcca19d8ff5529636b1b753e18';
 if(!fs.existsSync(source)||hash(fs.readFileSync(source))!==sourceSha){
  const response=await fetch('https://codeload.github.com/SagerNet/sing-box/tar.gz/refs/tags/v1.14.4',{signal:AbortSignal.timeout(120000)});
  if(!response.ok)throw new Error('sing-box source download failed');
  const bytes=Buffer.from(await response.arrayBuffer());if(hash(bytes)!==sourceSha)throw new Error('sing-box source digest mismatch');
  fs.mkdirSync(path.dirname(source),{recursive:true});fs.writeFileSync(source,bytes);
 }
 fs.writeFileSync(path.join(folder,'provenance.json'),JSON.stringify({version,archive:name,archiveSha256:digest,binarySha256:hash(bytes),source:`https://github.com/SagerNet/sing-box/tree/v${version}`},null,2));
 console.log(`Prepared pinned sing-box ${version}: ${platform}`);
 return folder;
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url))await prepare(process.argv[2]);
