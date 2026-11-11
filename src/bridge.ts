import { version } from '../package.json';
import { invoke, isTauri } from '@tauri-apps/api/core';
import {checkNativeUpdate, installNativeUpdate, restartUpdatedApp, type UpdateCheck, type UpdateProgress} from './app-updater';
export type {UpdateCheck,UpdateProgress} from './app-updater';
import { timezoneOffsetLabel, type TimezoneOption, type BrowserId, type CheckId, type Scan, type RepairRecord, type Outcome, type TimezoneTarget } from './domain';

export const native = isTauri();
let activeProfileId:string|null=null;
export function setActiveProfile(id:string|null){activeProfileId=id;}
export function getActiveProfile(){return activeProfileId;}
export async function getAutostartEnabled():Promise<boolean>{
  if(native)return invoke('get_autostart_enabled');
  return localStorage.getItem('nodecloak.autostart-demo')!=='false';
}
export async function setAutostartEnabled(enabled:boolean):Promise<boolean>{
  if(native)return invoke('set_autostart_enabled',{enabled});
  localStorage.setItem('nodecloak.autostart-demo',String(enabled));return enabled;
}
export async function checkForUpdates(): Promise<UpdateCheck> {
  if (native) return {...await checkNativeUpdate(),portable:await invoke<boolean>('is_portable_build')};
  await new Promise(resolve => setTimeout(resolve, 700));
  const preview = import.meta.env.DEV ? new URLSearchParams(location.search).get('updatePreview') : null;
  if (preview === 'offline') throw new Error('演示：连接官网失败，请检查网络后重试。');
  return { currentVersion: version, latestVersion: preview === 'available' ? '0.6.0' : version, available: preview === 'available', supported: !['unsupported','manual'].includes(preview ?? ''), manualDownload: preview === 'manual', notes: ['优化环境诊断体验（演示）', '改进浏览器配置兼容性（演示）'], publishedAt: new Date().toISOString(), checkedAt: new Date().toISOString() };
}
export async function installAvailableUpdate(onProgress:(progress:UpdateProgress)=>void):Promise<void> {
  if(native) return installNativeUpdate(onProgress);
  for(let downloaded=0;downloaded<=100;downloaded+=20) {
    onProgress({phase:'downloading',downloaded,total:100});
    await new Promise(resolve=>setTimeout(resolve,200));
  }
  onProgress({phase:'verifying',downloaded:100,total:100});
  await new Promise(resolve=>setTimeout(resolve,400));
  onProgress({phase:'installing',downloaded:100,total:100});
  await new Promise(resolve=>setTimeout(resolve,400));
  onProgress({phase:'ready',downloaded:100,total:100});
}
export async function restartAfterUpdate():Promise<void> {
  if(native) return restartUpdatedApp();
}
export async function openDownloadPage(): Promise<void> {
  if (native) return invoke('open_download_page');
  window.open('https://nodecloak.com/#download', '_blank', 'noopener,noreferrer');
}
export async function openFirefoxDownload(): Promise<void> {
  if (native) return invoke('open_firefox_download');
  window.open('https://www.firefox.com/en-US/download/all/desktop-release/', '_blank', 'noopener,noreferrer');
}
export interface FontItem { id: string; label: string; fileName: string; bytes: number }
export interface FontCatalog { items: FontItem[]; protected: string[]; note: string }
export interface FontOutcome { id: string; label: string; success: boolean; message: string }
const demoFonts:FontItem[]=[{id:'a'.repeat(64),label:'Noto Sans CJK SC · 用户安装（演示）',fileName:'NotoSansCJKsc-Regular.otf',bytes:17000000}];
export async function getFontCatalog():Promise<FontCatalog> {
  if(native) return invoke('get_font_catalog');
  return {items:structuredClone(demoFonts),protected:['Microsoft YaHei · msyh.ttc','SimSun / NSimSun · simsun.ttc'],note:'演示清单。桌面版只处理当前用户目录内识别到的字体，系统字体保留。'};
}
export async function removeUserFonts(browser:BrowserId,ids:string[],consent:boolean):Promise<FontOutcome[]> {
  if(native) return invoke('remove_user_fonts',{browser,ids,consent});
  if(!consent) throw new Error('卸载用户字体需要明确确认');
  await wait();
  return ids.map(id=>{const index=demoFonts.findIndex(f=>f.id===id);if(index<0)return {id,label:'所选字体',success:false,message:'字体已不在演示列表中'};const font=demoFonts.splice(index,1)[0];demoHistory.push({id:`${Date.now()}-fonts`,itemId:'fonts',browser,createdAt:new Date().toISOString(),status:'applied',message:'演示用户字体已卸载，系统字体保留，电脑未修改',changes:[{target:'demo',pointer:'fonts',before:font,after:null}]});return {id,label:font.label,success:true,message:'演示用户字体已卸载，系统字体保留；电脑设置没有改变。'};});
}
const wait = () => new Promise(resolve => setTimeout(resolve, 650));
const initial = (browser: BrowserId): Scan => ({
  platform: 'Windows', browser, browserAvailable: true, profilePath: '演示模式 · 独立浏览器配置目录',
  checkedAt: new Date().toISOString(), ip: '203.0.113.24', location: 'SG', latency: 328, cliInstalled: true,
  checks: [
    { id: 'connection', status: 'healthy', value: 'claude.ai HTTP 200 · claude.com HTTP 200', detail: '演示结果：本机 HTTPS 连接正常。实际使用时会测量两个域名的响应。', fixable: false },
    { id: 'route', status: 'manual', value: 'Cloudflare 出口 · SG（示例）', detail: '示例 IP 用于界面演示。Claude 的实际出口需要在专用浏览器中确认。', fixable: false },
    { id: 'webrtc', status: 'warning', value: '尚未设置隐私策略', detail: '专用浏览器尚未限制非代理 UDP，建议设置后做网页实测。', fixable: true },
    { id: 'dns', status: 'warning', value: '未配置加密 DNS', detail: '将专用浏览器设置为严格 DNS over HTTPS，保护域名查询。', fixable: true },
    { id: 'language', status: 'warning', value: 'zh-CN, zh, en-US', detail: '检测到中文语言偏好，专用环境可以单独切换为 English。', fixable: true },
    { id: 'timezone', status: 'warning', value: 'China Standard Time · UTC+8', detail: '可选调整为新加坡时区；这会影响系统内所有应用。', fixable: true },
    { id: 'offset', status: 'healthy', value: 'UTC+8', detail: '这是本机系统时区对应的 UTC 偏移。UTC+8 本身不是异常，也不能判断时钟是否准确；请查看系统时钟检测。', fixable: false },
    {id:'clock',status:'unknown',value:'时间准确性尚未确认',detail:'演示模式不进行实际时间校验。时区正常不代表电脑时间准确。',fixable:false},
    { id: 'locale', status: 'manual', value: '需要在专用浏览器中实测', detail: '请打开本地复检页读取 Intl 默认 locale，再导入报告。', fixable: false },
    { id: 'cli', status: 'configured', value: '专用启动器已准备', detail: '启动器使用英文 locale 和新加坡进程时区，启动后仍需验证实际环境。', fixable: true },
    { id: 'fonts', status: 'manual', value: '发现 2 个中文字体文件', detail: '建议保留系统字体，可在浏览器中进一步确认字体检测结果。', fixable: false },
    { id:'tracking',status:'warning',value:'DNT 尚未开启 · GPC 待网页复检',detail:'隐私偏好不保证停止跟踪。',fixable:true },
    ...(['webgl','screen','networkInfo','plugins'] as const).map(id => ({id,status:'manual' as const,value:'需要在专用浏览器中实测',detail:'仅网页采集，不能根据设备特征判断账号风险。',fixable:false})),
    { id: 'emoji', status: 'manual', value: '需要在浏览器中检测', detail: '请进入 NodeCloak 官网的环境检测页，查看当前浏览器的平台风格。', fixable: false },
  ],
});
function demoFor(browser:BrowserId):Scan {const key=activeProfileId??browser;if(!demos[key]){demos[key]=initial(browser);if(browser==='firefox'){const fonts=demos[key].checks.find(c=>c.id==='fonts')!;fonts.fixable=true;fonts.value='按需限制字体 · 保留电脑字体';}}return demos[key];}
const demos:Record<string,Scan> = { chrome: initial('chrome'), edge: initial('edge'), firefox: initial('firefox') };
Object.assign(demos.firefox.checks.find(c => c.id === 'fonts')!, {value:'按需限制字体 · 保留电脑字体', detail:'仅限制专用 Firefox 可用的系统字体，部分中文显示可能变化，电脑字体保留。', fixable:true});
Object.assign(demos.firefox.checks.find(c => c.id === 'webrtc')!, {detail:'关闭此专用 Firefox 的 WebRTC，网页音视频通话可能不可用。'});
Object.assign(demos.firefox.checks.find(c => c.id === 'tracking')!, {value:'GPC 尚未配置 · 待网页复检'});
let demoHistory: RepairRecord[] = [];

export async function scanEnvironment(browser: BrowserId): Promise<Scan> {
  if (native) return invoke('scan_environment', { browser,profileId:activeProfileId });
  await wait();
  demoFor(browser).checkedAt = new Date().toISOString();
  const result = structuredClone(demoFor(browser));
  if (import.meta.env.DEV && browser === 'firefox' && new URLSearchParams(location.search).get('browserPreview') === 'missing') result.browserAvailable = false;
  return result;
}
export async function getHistory(): Promise<RepairRecord[]> {
  if (native) return invoke('get_history',{profileId:activeProfileId});
  return structuredClone(demoHistory.filter(r=>!activeProfileId||r.profileId===activeProfileId));
}
export async function checkRepairReadiness(browser: BrowserId, ids: CheckId[]): Promise<{ ready: boolean; message: string }> {
  if (native) return invoke('check_repair_readiness', { browser, ids,profileId:activeProfileId });
  const blocked = import.meta.env.DEV && new URLSearchParams(location.search).get('repairPreview') === 'blocked';
  return { ready: !blocked || !ids.some(id => ['webrtc', 'dns', 'language', 'tracking', 'timezone', 'fonts'].includes(id)), message: '专用浏览器仍在运行。请保存未完成的输入，从该专用窗口的浏览器菜单选择“退出”，再回来继续。' };
}
export async function openTimezoneSettings(): Promise<void> {
  if (native) return invoke('open_timezone_settings');
  throw new Error('系统设置入口需要桌面版。当前为交互演示，未修改电脑。');
}
export async function openTelegramGroup(): Promise<void> {
  if (native) return invoke('open_telegram_group');
  window.open('https://t.me/nodecloak_official', '_blank', 'noopener,noreferrer');
}
export async function getTimezoneCatalog(): Promise<TimezoneOption[]> {
  if (native) return invoke('get_timezone_catalog');
  return [
    { id: 'UTC', label: '协调世界时', offsetMinutes: 0 },
    { id: 'Singapore Standard Time', label: '新加坡', offsetMinutes: -480 },
    { id: 'Tokyo Standard Time', label: '东京', offsetMinutes: -540 },
    { id: 'India Standard Time', label: '印度', offsetMinutes: -330 },
    { id: 'Eastern Standard Time', label: '美国东部（演示偏移）', offsetMinutes: 240 },
  ];
}
export async function repairEnvironment(browser: BrowserId, ids: CheckId[], consentTimezone: boolean, timezoneTarget: TimezoneTarget = 'singapore', consentFonts = false, customTimezone = ''): Promise<Outcome[]> {
  if (native) return invoke('repair_environment', { browser, ids,profileId:activeProfileId, consentTimezone, timezoneTarget: timezoneTarget === 'custom' ? `custom:${customTimezone}` : timezoneTarget, consentFonts });
  if (ids.includes('fonts') && (browser !== 'firefox' || !consentFonts)) throw new Error('专用 Firefox 字体限制需要明确勾选确认');
  if (ids.includes('timezone') && !consentTimezone) throw new Error('修改系统时区需要明确勾选确认');
  const zone = timezoneTarget === 'custom' ? (await getTimezoneCatalog()).find(zone => zone.id === customTimezone) : null;
  if (ids.includes('timezone') && timezoneTarget === 'custom' && !zone) throw new Error('请从系统支持的清单选择有效时区');
  const offset = zone?.offsetMinutes ?? (timezoneTarget === 'utc' ? 0 : -480);
  await wait();
  return ids.map(id => {
    if (import.meta.env.DEV && new URLSearchParams(location.search).get('repairPreview') === 'partial' && id !== 'cli') return { id, success: false, message: id === 'timezone' ? '演示：已取消管理员授权，可重试或到系统设置调整。' : '演示：专用浏览器在检查后重新启动，设置未写入。请退出后重试。' };
    const c = demoFor(browser).checks.find(c => c.id === id)!;
    const previous = structuredClone(c);
    const previousOffset = structuredClone(demoFor(browser).checks.find(c => c.id === 'offset')!);
    const after = { ...c, status: id === 'timezone' ? 'healthy' as const : 'configured' as const, value: ({ webrtc: '已限制非代理 UDP', dns: 'Cloudflare · 加密 DNS', language: 'en-US,en', tracking: 'DNT 已配置 · GPC 待网页复检', timezone: `${zone?.id ?? (timezoneTarget === 'utc' ? 'UTC' : 'Singapore Standard Time')} · ${timezoneOffsetLabel(offset)}`, cli: '专用启动器已准备' } as Partial<Record<CheckId, string>>)[id] || c.value };
    if (browser === 'firefox') {
      if(id === 'fonts') after.value = '字体允许列表已配置 · 待网页复检';
      if(id === 'tracking') after.value = 'GPC 已配置 · 待网页复检';
      if(id === 'webrtc') after.value = '专用 Firefox 已配置关闭 WebRTC';
    }
    Object.assign(c, after);
    if (id === 'timezone' || id === 'cli') {
      for (const state of Object.values(demos)) Object.assign(state.checks.find(c => c.id === id)!, after);
    }
    const changes: RepairRecord['changes'] = [{ target: 'demo', pointer: id, before: previous, after }];
    if (id === 'timezone') {
      const afterOffset = { ...previousOffset, status: 'healthy' as const, fixable: false, value: timezoneOffsetLabel(offset) };
      for (const state of Object.values(demos)) Object.assign(state.checks.find(c => c.id === 'offset')!, afterOffset);
      changes.push({ target: 'demo', pointer: 'offset', before: previousOffset, after: afterOffset });
    }
    demoHistory.push({ id: `${Date.now()}-${id}`, itemId: id, browser,profileId:['timezone','cli'].includes(id)?undefined:activeProfileId??`legacy-${browser}`, createdAt: new Date().toISOString(), status: 'applied', message: '演示修复完成，未修改电脑设置', changes });
    return { id, success: true, message: '演示设置已更新' };
  });
}
export async function undoRepair(recordId: string,profileId?:string): Promise<void> {
  if (native) return invoke('undo_repair', { recordId,profileId:profileId??null });
  await wait();
  const index = demoHistory.findIndex(r => r.id === recordId);
  const r = demoHistory[index];
  if (!r || r.status === 'undone') throw new Error('记录不存在或已撤销');
  if(r.itemId==='fonts' && r.browser!=='firefox') { demoFonts.push(r.changes[0].before as FontItem);r.status='undone';r.message='演示字体已恢复';return; }
  if (demoHistory.slice(index + 1).some(newer => newer.status !== 'undone' && newer.itemId === r.itemId && (['timezone', 'cli'].includes(r.itemId) || newer.browser === r.browser))) throw new Error('请先撤销这个项目较新的记录');
  const original=demos[r.profileId??r.browser]??demos[r.browser];Object.assign(original.checks.find(c => c.id === r.itemId)!, r.changes[0].before);
  if (['timezone', 'cli'].includes(r.itemId)) {
    for (const state of Object.values(demos)) Object.assign(state.checks.find(c => c.id === r.itemId)!, r.changes[0].before);
  }
  if (r.itemId === 'timezone') {
    const savedOffset = r.changes.find(c => c.pointer === 'offset');
    if (savedOffset) for (const state of Object.values(demos)) Object.assign(state.checks.find(c => c.id === 'offset')!, savedOffset.before);
  }
  r.status = 'undone'; r.message = '演示设置已恢复';
}
export type BrowserDestination = 'claude' | 'verify' | 'dns' | 'diagnostics' | 'privacyDocs' | `site:${string}`;
export async function launchBrowser(browser: BrowserId, destination: BrowserDestination) {
  if (native) return invoke<void>('launch_browser', { browser, destination,profileId:activeProfileId });
  throw new Error('专用浏览器需要桌面版才能打开。当前为界面演示，未更改电脑。');
}
export async function launchCli() {
  if (native) return invoke<void>('launch_cli');
  throw new Error('Claude Code 启动器需要桌面版才能运行。当前为界面演示。');
}

