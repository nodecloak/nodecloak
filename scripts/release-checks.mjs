import test from 'node:test';
// Run with node --test; keep separate from the frontend Vitest suites.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {releaseVersion,packageNames,updaterAssets,verifyReleaseAssets,desktopBundles,releaseFiles,updaterPackages} from './release-utils.mjs';

test('a mismatched tag or native version blocks publication',t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'claudedone-version-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  fs.mkdirSync(path.join(root,'src-tauri'));
  fs.writeFileSync(path.join(root,'package.json'),JSON.stringify({version:'0.5.8'}));
  fs.writeFileSync(path.join(root,'src-tauri/tauri.conf.json'),JSON.stringify({version:'0.5.8'}));
  fs.writeFileSync(path.join(root,'src-tauri/Cargo.toml'),'[package]\nversion = "0.5.8"\n[dependencies]\n');
  assert.equal(releaseVersion(root,'v0.5.8'),'0.5.8');
  assert.throws(()=>releaseVersion(root,'v0.5.9'),/does not match/);
  fs.writeFileSync(path.join(root,'src-tauri/Cargo.toml'),'[package]\nversion = "0.5.7"\n');
  assert.throws(()=>releaseVersion(root,'v0.5.8'),/versions must match/);
});

test('incomplete, stale or empty asset sets cannot be released',t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'claudedone-assets-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const names=[...packageNames('0.5.8'),...updaterAssets('0.5.8'),'NodeCloak_0.5.8_THIRD_PARTY_NOTICES.tar.gz'];
  for(const name of names.slice(1)) fs.writeFileSync(path.join(root,name),'package');
  assert.throws(()=>verifyReleaseAssets(root,'0.5.8'),/all six/);
  fs.writeFileSync(path.join(root,names[0]),'package');
  assert.equal(verifyReleaseAssets(root,'0.5.8').length,13);
  fs.writeFileSync(path.join(root,'stale.exe'),'package');
  assert.throws(()=>verifyReleaseAssets(root,'0.5.8'),/unexpected/);
  fs.unlinkSync(path.join(root,'stale.exe'));
  fs.writeFileSync(path.join(root,names[0]),'');
  assert.throws(()=>verifyReleaseAssets(root,'0.5.8'),/empty/);
});

test('Linux publishes deb and AppImage while the updater uses the AppImage',()=>{
  const version='5.0.2';
  assert.equal(desktopBundles('linux'),'deb,appimage');
  assert.throws(()=>desktopBundles('freebsd'),/Unsupported/);
  assert.deepEqual(releaseFiles('linux-x64',version,true),[
    ['bundle/deb/NodeCloak_5.0.2_amd64.deb','NodeCloak_5.0.2_linux_x64.deb'],
    ['bundle/appimage/NodeCloak_5.0.2_amd64.AppImage','NodeCloak_5.0.2_linux_x64.AppImage'],
    ['bundle/appimage/NodeCloak_5.0.2_amd64.AppImage.sig','NodeCloak_5.0.2_linux_x64.AppImage.sig'],
  ]);
  assert.equal(releaseFiles('linux-x64',version).length,2);
  const collected=['windows-x64','macos-arm64','macos-x64','linux-x64'].flatMap(platform=>releaseFiles(platform,version,true).map(([,name])=>name)).sort();
  assert.deepEqual(collected,[...packageNames(version),...updaterAssets(version)].sort());
  assert.equal(updaterPackages(version)['linux-x86_64'],'NodeCloak_5.0.2_linux_x64.AppImage');
  assert.equal(Object.keys(updaterPackages(version)).length,4);
});

test('a missing Linux updater signature blocks an otherwise complete release',t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'nodecloak-linux-assets-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const version='5.0.2';
  for(const name of [...packageNames(version),...updaterAssets(version),`NodeCloak_${version}_THIRD_PARTY_NOTICES.tar.gz`]) fs.writeFileSync(path.join(root,name),'package');
  fs.unlinkSync(path.join(root,`NodeCloak_${version}_linux_x64.AppImage.sig`));
  assert.throws(()=>verifyReleaseAssets(root,version),/signed updater bundles/);
});
