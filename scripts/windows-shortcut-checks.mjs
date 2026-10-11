import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';

// Exercise the installed NSIS code, with shell folders redirected to a temporary
// directory. Never install the app or write to the user's Start menu/registry.
const windows = process.platform === 'win32';
const root = path.resolve(import.meta.dirname, '..');
const generated = process.env.NSIS_GENERATED_DIR || path.join(root, 'src-tauri/target/release/nsis/x64');
const compiler = process.env.NSIS_COMPILER || path.join(process.env.LOCALAPPDATA || '', 'tauri/NSIS/makensis.exe');
const quote = value => `'${value.replaceAll("'", "''")}'`;
function run(command, args, cwd) {
  const result = spawnSync(command, args, {cwd, encoding: 'utf8', timeout: 60000, windowsHide: true});
  assert.equal(result.status, 0, result.error?.message || result.stdout + result.stderr);
  return result.stdout.trim();
}

for (const scenario of [
  {name: 'update restores a missing Start menu entry', update: true, existing: false, skip: false, expected: true},
  {name: 'update preserves an existing customized shortcut', update: true, existing: true, skip: false, expected: true},
  {name: 'explicit no-shortcuts install does not create an entry', update: true, existing: false, skip: true, expected: false},
  {name: 'fresh install creates a Start menu entry', update: false, existing: false, skip: false, expected: true},
]) {
  test(scenario.name, {skip: !windows}, t => {
    const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'nodecloak-shortcut-'));
    t.after(() => fs.rmSync(temporary, {recursive: true, force: true}));
    const programs = path.join(temporary, 'Start menu');
    const app = path.join(temporary, 'Installed app');
    fs.mkdirSync(programs);
    fs.mkdirSync(app);
    const target = path.join(app, 'nodecloak.exe');
    const link = path.join(programs, 'NodeCloak.lnk');
    fs.writeFileSync(target, 'fixture executable, never launched');
    if (scenario.existing) {
      run('powershell.exe', ['-NoProfile', '-Command', `$s=(New-Object -ComObject WScript.Shell).CreateShortcut(${quote(link)}); $s.TargetPath=${quote(target)}; $s.Arguments='--retained'; $s.Save()`], temporary);
    }
    const original = scenario.existing ? fs.readFileSync(link) : null;
    const installer = fs.readFileSync(path.join(generated, 'installer.nsi'), 'utf8');
    const create = installer.match(/Function CreateOrUpdateStartMenuShortcut\r?\n[\s\S]*?FunctionEnd/);
    assert.ok(create, 'Tauri-generated shortcut function must be present');
    const config = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
    const hook = config.bundle.windows?.nsis?.installerHooks;
    // NSIS shell folders are constants: redirect only their path literals in
    // these fixture copies; retain the production hook/function control flow.
    const redirect = source => source.replaceAll('$SMPROGRAMS', programs);
    if (hook) fs.writeFileSync(path.join(temporary, 'hook.nsh'), redirect(fs.readFileSync(path.resolve(root, 'src-tauri', hook), 'utf8')));
    fs.writeFileSync(path.join(temporary, 'fixture.nsi'), `Unicode true
!include LogicLib.nsh
!include "Win\\COM.nsh"
!include "Win\\Propkey.nsh"
!include "${path.join(generated, 'utils.nsh')}"
!define PRODUCTNAME "NodeCloak"
!define MAINBINARYNAME "nodecloak"
!define BUNDLEID "com.nodecloak"
!define STARTMENUFOLDER ""
Var AppStartMenuFolder
Var OldMainBinaryName
Var WixMode
Var UpdateMode
Var NoShortcutMode
${hook ? '!include "hook.nsh"' : ''}
Name "NodeCloak shortcut fixture"
OutFile "fixture.exe"
RequestExecutionLevel user
SilentInstall silent
Section
  StrCpy $INSTDIR "${app}"
  StrCpy $AppStartMenuFolder ""
  StrCpy $OldMainBinaryName "previous-name.exe"
  StrCpy $WixMode 0
  StrCpy $UpdateMode ${scenario.update ? 1 : 0}
  StrCpy $NoShortcutMode ${scenario.skip ? 1 : 0}
  Call CreateOrUpdateStartMenuShortcut
  !ifmacrodef NSIS_HOOK_POSTINSTALL
    !insertmacro NSIS_HOOK_POSTINSTALL
  !endif
SectionEnd
${redirect(create[0])}
`);
    run(compiler, ['/V2', 'fixture.nsi'], temporary);
    run('powershell.exe', ['-NoProfile', '-Command', `$p=Start-Process -FilePath ${quote(path.join(temporary, 'fixture.exe'))} -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode`], temporary);
    assert.equal(fs.existsSync(link), scenario.expected, 'Start menu shortcut presence');
    if (!scenario.expected) return;
    if (original) assert.deepEqual(fs.readFileSync(link), original, 'Existing shortcut must not be rewritten');
    const shortcut = JSON.parse(run('powershell.exe', ['-NoProfile', '-Command', `$s=(New-Object -ComObject WScript.Shell).CreateShortcut(${quote(link)}); $f=(New-Object -ComObject Shell.Application).NameSpace(${quote(programs)}).ParseName('NodeCloak.lnk'); @{Target=$s.TargetPath; AppId=$f.ExtendedProperty('System.AppUserModel.ID')} | ConvertTo-Json -Compress`], temporary));
    // Windows shell links expand DOS 8.3 names (e.g. runner~1); compare the
    // actual files, rather than rejecting two spellings of the same path.
    assert.equal(fs.realpathSync.native(shortcut.Target).toLowerCase(), fs.realpathSync.native(target).toLowerCase());
    if (!original) assert.equal(shortcut.AppId, 'com.nodecloak');
  });
}
