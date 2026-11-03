import {t} from './i18n';
import { useEffect, useRef, useState, type ElementType } from 'react';
import { Activity, ArrowRight, ArrowUpRight, BookOpen, Check, CheckCheck, ChevronDown, ChevronRight, CircleHelp, Clock3, Fingerprint, Globe2, History, Info, Languages, LayoutDashboard, LoaderCircle, LockKeyhole, Monitor, RefreshCw, RotateCcw, Settings2, ShieldCheck, Sparkles, Terminal, Type, Wifi, X, CircleAlert, Send } from 'lucide-react';
import { checkCount, defaultTimezoneTarget, definitions, labels, recommendedChecks, repairIds, timezoneChoices, timeLabel, type BrowserId, type CheckId, type Check as EnvironmentCheck, type Outcome, type RepairRecord, type Scan, type TimezoneTarget, type TimezoneOption } from './domain';
import { parseBrowserReport, withBrowserReport, type BrowserReport } from './browser-report';
import AppUpdates from './AppUpdates';
import AutostartSettings from './AutostartSettings';
import {createEnvironmentCache,environmentScopeKey,type EnvironmentScope} from './environment-cache';
import ThemeControl from './ThemeControl';
import McpWorkspace from './McpWorkspace';
import ApplicationProxy,{ApplicationProxyIndicator} from './ApplicationProxy';
import {Network} from 'lucide-react';
import {PlugZap,Cloud} from 'lucide-react';
import {useCloudAccount,AccountButton,AccountDialog,CloudSettings} from './AccountWorkspace';
import {isSyncRecoveryError} from './cloud';
import WindowControls from './WindowControls';
import TerminalWorkspace from './TerminalWorkspace';
import {useLanguage} from './LanguageControl';
import BrowserProfiles from './BrowserProfiles';
import {ShareProfileDialog,SharingWorkspace,useProfileSharing} from './ProfileSharing';
import {effectiveShareStatus} from './profile-sharing';
import {Share2} from 'lucide-react';
import BrowserIcon from './BrowserIcon';
import BrowserSiteShortcuts from './BrowserSiteShortcuts';
import AnnouncementCenter from './AnnouncementCenter';
import {openCatalogSupport,useBrowserSiteCatalog} from './browser-site-catalog';
import SelectMenu from './SelectMenu';
import {listProfiles,quitApplication,type BrowserProfile} from './browser-profiles';
import {setActiveProfile} from './bridge';
import {listen} from '@tauri-apps/api/event';
import {version} from '../package.json';
import TimezonePicker from './TimezonePicker';
import FirefoxSetup from './FirefoxSetup';
import { FontReview, FontResult } from './FontDialogs';
import { getFontCatalog, removeUserFonts, type FontCatalog, type FontOutcome } from './bridge';
import { summarizeOutcomes } from './repair-result';
import { getTimezoneCatalog, checkRepairReadiness, openTimezoneSettings, openFirefoxDownload, native, type BrowserDestination, scanEnvironment, getHistory, repairEnvironment, undoRepair, launchBrowser } from './bridge';

type Page = 'profiles' | 'sharing' | 'computer' | 'overview' | 'code' | 'history' | 'settings' | 'account' | 'devices' | 'automation' | 'sync' | 'appProxy';
type RepairRequest = { browser: BrowserId; ids: CheckId[]; consentTimezone: boolean; consentFonts: boolean; timezoneTarget: TimezoneTarget; customTimezone: string };
type Modal = {kind:'fonts';catalog:FontCatalog;initialIds:string[]} | {kind:'font-result';outcomes:FontOutcome[]} | { kind: 'repair'; ids: CheckId[] } | { kind: 'detail'; id: CheckId } | { kind: 'undo'; record: RepairRecord } | { kind: 'help' } | { kind: 'result'; outcomes: Outcome[]; request: RepairRequest } | { kind: 'blocked'; request: RepairRequest; reason: string } | { kind: 'report' };
const icons: Record<CheckId, ElementType> = { connection: Wifi, route: Globe2, webrtc: ShieldCheck, dns: LockKeyhole, language: Languages, timezone: Clock3, offset: Clock3, clock: Clock3, locale: Languages, cli: Terminal, fonts: Type, emoji: Fingerprint, webgl: Monitor, screen: Monitor, networkInfo: Wifi, plugins: Settings2, tracking: ShieldCheck };
const nav: { id: Page; title: string; icon: ElementType }[] = [
  {id:'profiles',title:'浏览器副本',icon:Globe2},{id:'appProxy',title:'应用代理',icon:Network},{id:'sync',title:'云端同步',icon:Cloud},{id:'sharing',title:'共享',icon:Share2},{id:'automation',title:'MCP',icon:PlugZap},{id:'computer',title:'电脑环境',icon:Monitor},
  { id: 'overview', title: '环境概览', icon: LayoutDashboard }, { id: 'code', title: '终端', icon: Terminal }, { id: 'history', title: '修复记录', icon: History },
];
const pageTitles: Record<Page, string> = {
  appProxy:'应用代理',
  automation:'MCP 与自动化',
  sync:'云端同步',
  profiles: '浏览器副本', sharing:'共享', computer: '电脑环境', overview: '环境概览', code: '终端', history: '修复记录', settings: '偏好设置', account:'个人资料',devices:'设备',
};
const browserNames = { chrome: 'Google Chrome', edge: 'Microsoft Edge', firefox: 'Firefox' };
function Mark({ small = false }: { small?: boolean }) { return <span className={small ? 'brand-mark small' : 'brand-mark'}><img className="mark-light" src="/brand/symbol-positive.svg" alt="" /><img className="mark-dark" src="/brand/symbol-primary.svg" alt="" /></span>; }
function Status({ status, id }: { status: EnvironmentCheck['status']; id?:CheckId }) { const label=status==='healthy'&&id==='timezone'?'已读取时区':status==='healthy'&&id==='offset'?'已读取偏移':labels[status]; return <span className={`status status-${status}`}><span />{t(label)}</span>; }
function Orbit({ count, total }: { count: number; total: number }) {
  return <div className="orbit" aria-label={t(`${count} 项建议调整`)}>
    <svg viewBox="0 0 190 190" aria-hidden="true"><circle cx="95" cy="95" r="79" className="orbit-track" /><circle cx="95" cy="95" r="79" className="orbit-progress" strokeDasharray={`${(count / total) * 496} 496`} /><circle cx="95" cy="95" r="61" className="orbit-inner" />{Array.from({ length: 36 }, (_, i) => <line key={i} x1="95" y1="41" x2="95" y2="45" transform={`rotate(${i * 10} 95 95)`} />)}</svg>
    <div className="orbit-number">{count}<span>{t("项建议调整")}</span></div>
    <span className="orbit-star"><Sparkles size={16} /></span>
  </div>;
}

export default function App() {
  useLanguage();
  const {catalog:siteCatalog}=useBrowserSiteCatalog();
  const [announcementToolbar,setAnnouncementToolbar]=useState<HTMLSpanElement|null>(null);
  const cloud=useCloudAccount();
  const [syncTarget,setSyncTarget]=useState<string>();
  function openSyncConflict(id?:string){setSyncTarget(id);setPage('sync');cloud.setPanel(null);void cloud.reload();}
  const sharing=useProfileSharing(cloud.status.account?.id);
  const [shareProfile,setShareProfile]=useState<BrowserProfile|null>(null);
  const [page, setPage] = useState<Page>('profiles');
  useEffect(()=>{if(cloud.panel==='sync'){setPage('sync');cloud.setPanel(null);}},[cloud.panel]);
  useEffect(()=>{if(page!=='sync')setSyncTarget(undefined);},[page]);
  const [mcpPending,setMcpPending]=useState(false);
  useEffect(()=>{if(!native)return;let disposed=false;let off:(()=>void)|undefined;void listen<{pending:number}>('mcp-approval',event=>setMcpPending(event.payload.pending>0)).then(fn=>{if(disposed)fn();else off=fn;});return()=>{disposed=true;off?.();};},[]);
  useEffect(()=>{if((page==='account'||page==='devices')&&!cloud.status.account)setPage('profiles');},[page,cloud.status.account?.id]);
  const [activeProfile,setActiveProfileState]=useState<BrowserProfile|null>(null);
  const [profileOptions,setProfileOptions]=useState<BrowserProfile[]>([]);
  const [quitting,setQuitting]=useState(false);
  const [quitError,setQuitError]=useState('');
  const quitDialog=useRef<HTMLDialogElement>(null);
  const [browser, setBrowser] = useState<BrowserId>(() => (['edge','firefox'].includes(localStorage.getItem('claude-ready-browser') || '') ? localStorage.getItem('claude-ready-browser') : 'chrome') as BrowserId);
  const reportKey=activeProfile?.id??browser;
  const [scan, setScan] = useState<Scan | null>(null);
  const observationCache=useRef<ReturnType<typeof createEnvironmentCache>|null>(null);
  if(!observationCache.current)observationCache.current=createEnvironmentCache();
  const currentScope=useRef<EnvironmentScope>({browser,profileId:null,native});
  const navigationGeneration=useRef(0);
  const historyRequests=useRef(new Map<string,Promise<void>>());
  const historyGeneration=useRef(0);
  const [history, setHistory] = useState<RepairRecord[]>([]);
  const [historyLoading,setHistoryLoading]=useState(new Set<string>());
  const [busy, setBusy] = useState<string | null>(null);
  const [modal, setModal] = useState<Modal | null>(null);
  const [selected, setSelected] = useState<CheckId[]>([]);
  const [consentFonts, setConsentFonts] = useState(false);
  const [consentTimezone, setConsentTimezone] = useState(false);
  const [timezoneTarget, setTimezoneTarget] = useState<TimezoneTarget>('singapore');
  const [customTimezone, setCustomTimezone] = useState('');
  const [timezoneCatalog, setTimezoneCatalog] = useState<TimezoneOption[]>([]);
  const [timezoneLoading, setTimezoneLoading] = useState(false);
  const [timezoneError, setTimezoneError] = useState('');
  const timezoneLabel = timezoneTarget === 'custom' ? customTimezone || '自定义时区（尚未选择）' : timezoneChoices[timezoneTarget].label;
  const customTimezoneValid = timezoneTarget !== 'custom' || timezoneCatalog.some(zone => zone.id === customTimezone);
  async function loadTimezoneCatalog() {
    setTimezoneLoading(true); setTimezoneError('');
    try { setTimezoneCatalog(await getTimezoneCatalog()); }
    catch (error) { setTimezoneError(String(error)); }
    finally { setTimezoneLoading(false); }
  }
  useEffect(() => {
    if (timezoneTarget === 'custom' && modal?.kind === 'repair' && !timezoneCatalog.length && !timezoneError) void loadTimezoneCatalog();
  }, [timezoneTarget, modal?.kind]);
  const [reports, setReports] = useState<Record<string, BrowserReport | null>>({});
  const [reportInput, setReportInput] = useState('');
  const [group, setGroup] = useState('all');
  const [onlyPending, setOnlyPending] = useState(false);
  const [toast, setToast] = useState<{ text: string; error?: boolean; syncRecovery?:boolean; syncProfileId?:string } | null>(null);
  const initialized = useRef(false);
  const dialogRef = useRef<HTMLDialogElement>(null);
  const allChecks=withBrowserReport(scan,reports[reportKey],page==='computer'?'computer':'browser')?.checks||[];
  const checks=allChecks.filter(c=>page==='computer'?['timezone','offset','clock'].includes(c.id):!['timezone','offset','cli'].includes(c.id)&&!(c.id==='fonts'&&browser!=='firefox'));
  const warnings = checks.filter(c => c.status === 'warning');
  const configured = checks.filter(c => c.status === 'configured').length;
  const healthy = checks.filter(c => c.status === 'healthy').length;
  const manual = checks.filter(c => ['manual', 'unknown'].includes(c.status)).length;
  const suggested = recommendedChecks(checks);
  const pendingCount = warnings.length + manual;
  const displayed = checks.filter(c => (group === 'all' || definitions[c.id].group === group) && (!onlyPending || ['warning', 'manual', 'unknown'].includes(c.status)));

  function notify(text: string, error = false) { setToast({ text, error }); }
  async function loadProfileOptions() {
    const list = (await listProfiles()).filter(p=>!p.deletedAt);
    setProfileOptions(list);
    return list;
  }
  async function refreshHistory(scope=currentScope.current) {
    const key=environmentScopeKey(scope),pending=historyRequests.current.get(key);
    if(pending)return pending;
    const generation=historyGeneration.current;
    setHistoryLoading(current=>new Set(current).add(key));
    const request=(async()=>{
      const records=(await getHistory()).sort((a,b)=>b.createdAt.localeCompare(a.createdAt));
      if(generation!==historyGeneration.current)return;
      observationCache.current!.saveHistory(scope,records);
      if(environmentScopeKey(currentScope.current)===key)setHistory(records);
    })();
    historyRequests.current.set(key,request);
    try{await request;}finally{if(historyRequests.current.get(key)===request){historyRequests.current.delete(key);setHistoryLoading(current=>{const next=new Set(current);next.delete(key);return next;});}}
  }
  function invalidateHistory(){
    historyGeneration.current++;historyRequests.current.clear();
    observationCache.current!.clearHistories();setHistoryLoading(new Set());
  }
  function handleProfileReset(p:BrowserProfile) {
    const scope={browser:p.browser,profileId:p.id,native};
    observationCache.current!.forget(scope);
    setReports(current=>{const next={...current};delete next[p.id];return next;});
    if(currentScope.current.profileId===p.id){navigationGeneration.current++;historyGeneration.current++;setScan(null);setHistory([]);}
    void loadProfileOptions().then(list=>{if(currentScope.current.profileId===p.id)setActiveProfileState(list.find(item=>item.id===p.id)??null);}).catch(error=>notify(String(error),true));
  }
  function restoreObservations(target:BrowserId,profileId:string|null) {
    const scope={browser:target,profileId,native};currentScope.current=scope;
    setScan(observationCache.current!.readScan(scope));
    setHistory(observationCache.current!.readHistory(scope)??[]);
    setGroup('all');setOnlyPending(false);
    return scope;
  }
  async function loadSavedHistory(scope:EnvironmentScope) {
    if(observationCache.current!.readHistory(scope)!==null)return;
    try{await refreshHistory(scope);}catch(error){notify(String(error),true);}
  }
  async function refresh(target = browser) {
    const scope={...currentScope.current,browser:target};
    const results = await Promise.allSettled([scanEnvironment(target), refreshHistory(scope)]);
    if (results[0].status === 'fulfilled') {observationCache.current!.saveScan(scope,results[0].value);if(environmentScopeKey(currentScope.current)===environmentScopeKey(scope))setScan(results[0].value);} else notify(String(results[0].reason), true);
    if (results[1].status === 'rejected')notify(String(results[1].reason), true);
  }
  async function perform(label: string, action: () => Promise<void>, syncProfileId?:string) {
    if (busy) return;
    setBusy(label);
    try { await action(); } catch (e) { const text=e instanceof Error?e.message:String(e);if(isSyncRecoveryError(text)){setModal(null);setToast({text,error:true,syncRecovery:true,syncProfileId});void cloud.reload();}else notify(text,true); }
    finally { setBusy(null); }
  }
  useEffect(() => {
    if (!initialized.current) {initialized.current=true;void loadProfileOptions().then(list=>{if(navigationGeneration.current!==0)return;const saved=localStorage.getItem('claudedone.active-profile');const p=list.find(p=>p.id===saved)||list.find(p=>p.id===`legacy-${browser}`);if(p){setActiveProfile(p.id);setActiveProfileState(p);setBrowser(p.browser);restoreObservations(p.browser,p.id);}}).catch(e=>notify(String(e),true));}
    // Restore observations without scanning, including under StrictMode.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(()=>{if(quitting)quitDialog.current?.showModal();else quitDialog.current?.close();},[quitting]);
  useEffect(()=>{if(!native)return;let stop:(()=>void)|undefined;let cancelled=false;void listen('request-quit',()=>{setQuitError('');setQuitting(true);}).then(fn=>{if(cancelled)fn();else stop=fn;});return()=>{cancelled=true;stop?.();};},[]);
  async function selectProfile(p:BrowserProfile,showHistory=false){if(busy)return;navigationGeneration.current++;setActiveProfile(p.id);setActiveProfileState(p);setBrowser(p.browser);localStorage.setItem('claudedone.active-profile',p.id);const scope=restoreObservations(p.browser,p.id);setPage(showHistory?'history':'overview');if(showHistory)await loadSavedHistory(scope);}
  useEffect(()=>{if(!native)return;let stopped=false;let stop:(()=>void)|undefined;void listen<{reason?:string}>('sharing-changed',event=>{const reason=event.payload?.reason;if(reason==='expiring')notify('共享副本将在 5 分钟内到期，请保存未完成的操作。');else if(reason==='authorizationEnded')notify('共享授权已结束，相关共享浏览器已关闭。');else if(reason==='authorizationFailed')notify('无法确认共享授权，相关共享浏览器已关闭。请检查网络并重新验证。',true);void loadProfileOptions().catch(e=>notify(String(e),true));}).then(fn=>{if(stopped)fn();else stop=fn;});return()=>{stopped=true;stop?.();};},[]);
  useEffect(()=>{if(page==='overview')void loadProfileOptions().catch(e=>notify(String(e),true));},[page,activeProfile?.id]);
  async function navigate(next:Page){
    if(busy)return;
    const generation=++navigationGeneration.current;
    if(next==='code'){setActiveProfile(null);setActiveProfileState(null);restoreObservations(browser,null);setPage(next);return;}
    if(next==='computer'){
      setActiveProfile(null);setActiveProfileState(null);restoreObservations(browser,null);setPage(next);return;
    }
    if(next==='overview'||next==='history'&&activeProfile){
      try{const saved=activeProfile?.id||localStorage.getItem('claudedone.active-profile');const list=await loadProfileOptions();const p=list.find(p=>p.id===saved)||list[0];
        if(generation!==navigationGeneration.current)return;
        if(!p){notify('请先创建浏览器副本。');setPage('profiles');return;}
        setActiveProfile(p.id);setActiveProfileState(p);setBrowser(p.browser);const scope=restoreObservations(p.browser,p.id);setPage(next);if(next==='history')await loadSavedHistory(scope);
      }catch(e){notify(String(e),true);}return;
    }
    setPage(next);if(next==='history')await loadSavedHistory(currentScope.current);
  }
  async function quit(){if(busy)return;setBusy('正在关闭副本并退出');try{await quitApplication(true);setQuitting(false);}catch(e){setQuitError(String(e));}finally{setBusy(null);}}
  useEffect(() => { if (toast&&!toast.syncRecovery) { const timer = setTimeout(() => setToast(null), toast.error ? 9000 : 5000); return () => clearTimeout(timer); } }, [toast]);
  useEffect(() => { if (modal) dialogRef.current?.showModal(); else dialogRef.current?.close(); }, [modal]);
  function repairDialog(ids: CheckId[], target: TimezoneTarget = defaultTimezoneTarget(ids), custom = '') { const mapped = repairIds(ids); setSelected(mapped); setConsentTimezone(false); setConsentFonts(false); setTimezoneTarget(target); setCustomTimezone(custom); setModal({ kind: 'repair', ids: mapped }); }
  async function fontDialog(initialIds: string[] = []) {
    if(browser === 'firefox'&&page!=='computer') { repairDialog(['fonts']); return; }
    await perform('正在读取用户字体清单', async () => {
      const catalog = await getFontCatalog();
      setModal({kind:'fonts',catalog,initialIds});
    });
  }
  async function applyFonts(ids: string[], consent: boolean) {
    await perform('正在备份并处理用户字体', async () => {
      const outcomes = await removeUserFonts(browser, ids, consent);
      invalidateHistory();
      if(outcomes.some(o => o.success)) setReports({});
      setModal({kind:'font-result',outcomes}); await refresh();
    });
  }
  function importReport() {
    try { const report = parseBrowserReport(reportInput);if(activeProfile&&!activeProfile.legacy&&!report.profileId)throw new Error('报告缺少副本标识，请从此副本重新打开检测页采集。');if(report.profileId&&report.profileId!==activeProfile?.id)throw new Error('这份报告来自另一个浏览器副本，请切换到对应副本再导入。'); if (report.browser && report.browser !== browser) throw new Error("这份报告来自另一个专用浏览器，请先切换浏览器再导入。"); setReports(prev => ({...prev, [reportKey]:report})); setModal(null); notify('已导入网页采集报告，检测项现在显示报告中的实际值。'); }
    catch (e) { notify(e instanceof Error ? e.message : String(e), true); }
  }
  function bulkRepair() {
    if (!suggested.length) { notify('自动修复项目已配置。你可以查看手动确认项，或打开网页复检。'); return; }
    repairDialog(suggested.map(c => c.id));
  }
  async function switchBrowser(next: BrowserId) {
    if (next === browser || busy) return;
    const p=(await listProfiles()).find(p=>p.id===`legacy-${next}`&&!p.deletedAt);if(p){await selectProfile(p);return;}notify('此浏览器的默认副本已删除，请在副本列表中新建或恢复。');setPage('profiles');
  }
  async function applyRepair(request: RepairRequest = { browser, ids: [...selected], consentTimezone, consentFonts, timezoneTarget, customTimezone }) {
    await perform('正在检查并应用设置', async () => {
      const readiness = await checkRepairReadiness(request.browser, request.ids);
      if (!readiness.ready) { setModal({ kind: 'blocked', request, reason: readiness.message }); return; }
      const outcomes = await repairEnvironment(request.browser, request.ids, request.consentTimezone, request.timezoneTarget, request.consentFonts, request.customTimezone);
      invalidateHistory();
      if (outcomes.some(o => o.success)) setReports(prev => outcomes.some(o => o.success && o.id === 'timezone') ? {} : {...prev, [reportKey]:null});
      setModal({ kind: 'result', outcomes, request });
      await refresh(request.browser);
    });
  }
  const openBrowser = (destination: BrowserDestination) => perform('正在打开专用浏览器', async () => { await launchBrowser(browser, destination); notify('已打开专用浏览器。请在该窗口内验证设置。'); },activeProfile?.id);

  function checkRow(c: EnvironmentCheck) {
    const Icon = icons[c.id];
    return <div className="check-row" key={c.id}>
      <button className="check-name" onClick={() => setModal({ kind: 'detail', id: c.id })} aria-label={t(`查看${definitions[c.id].title}详情`)}><span className={`check-icon ${definitions[c.id].group}`}><Icon size={19} strokeWidth={1.7} /></span><span><strong>{t(definitions[c.id].title)}</strong><small>{t(definitions[c.id].description)}</small></span></button>
      <span className="check-value" title={t(c.value)}>{t(c.value)}</span><Status status={c.status} id={c.id} />
      {c.id === 'clock' ? <button className="row-action" disabled={!!busy} onClick={() => void perform('正在打开系统设置', openTimezoneSettings)}>{t("校准时间")}<ArrowUpRight size={13} /></button> : c.id === 'fonts' ? <button className="row-action" disabled={!!busy} onClick={() => void fontDialog()}>{t(browser === 'firefox' ? '限制字体' : '管理字体')}<ArrowUpRight size={13} /></button> : c.fixable && (c.status === 'warning' || c.id === 'timezone' || c.id === 'offset') ? <button className="row-action" onClick={() => repairDialog([c.id])} disabled={!!busy}>{t(['timezone','offset'].includes(c.id) ? '设置时区' : '修复')}<ArrowUpRight size={13} /></button> : <button className="row-detail" onClick={() => setModal({ kind: 'detail', id: c.id })}>{t(c.status === 'configured' ? '复检说明' : c.status === 'healthy' ? '查看详情' : '处理建议')}<ChevronRight size={13} /></button>}
    </div>;
  }

  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand" data-tauri-drag-region="deep"><Mark /><span><span className="brand-node">Node</span><b>Cloak</b><small>BROWSER WORKSPACE</small></span></div>
      <span className="sidebar-label">{t("工作空间")}</span>
      <nav aria-label={t("主导航")}>{nav.map(({ id, title, icon: Icon }) => <button key={id} className={`nav-item ${page === id ? 'active' : ''}`} disabled={!!busy} onClick={() => void navigate(id)}><Icon size={18} strokeWidth={1.7} /><span>{t(title)}</span>{id==='appProxy'&&<ApplicationProxyIndicator/>}{id==='sync'&&!!cloud.status.conflicts?.length&&<span className="nav-count">{new Set(cloud.status.conflicts.map(c=>c.localId)).size}</span>}{id==='sharing'&&sharing.data.received.some(s=>effectiveShareStatus(s)==='pending')&&<span className="nav-count">{sharing.data.received.filter(s=>effectiveShareStatus(s)==='pending').length}</span>}{id === 'overview' && warnings.length > 0 && <span className="nav-count">{warnings.length}</span>}{id === 'history' && history.filter(r => r.status === 'applied').length > 0 && <span className="nav-history-dot" />}</button>)}</nav>
      <div className="sidebar-bottom"><div className="sidebar-footer"><AccountButton cloud={cloud} preferencesDisabled={!!busy} onPreferences={()=>{cloud.setPanel(null);setPage('settings');}} onAccountPage={next=>{cloud.setError('');cloud.setPanel(null);setPage(next);}}/><span>v{t(version)}</span></div></div>
    </aside>
    <main>
      <header className="topbar" data-tauri-drag-region="deep"><div className="breadcrumb">NodeCloak<ChevronRight size={12} /><span>{t(pageTitles[page])}</span></div><div className="topbar-right">{!native && <span className="demo-badge">{t("交互演示 · 不修改电脑")}</span>}{siteCatalog.supportURL&&<button className="telegram-group" title={siteCatalog.supportURL.startsWith('https://t.me/')?t("加入 Telegram 群"):t("帮助与说明")} disabled={!!busy} onClick={() => void perform('正在打开帮助', openCatalogSupport)}>{siteCatalog.supportURL.startsWith('https://t.me/')?<Send size={14}/>:<CircleHelp size={14}/>}<span>{siteCatalog.supportURL.startsWith('https://t.me/')?'Telegram Group':t('帮助与说明')}</span><ArrowUpRight size={13} /></button>}<span className="announcement-toolbar-slot" ref={setAnnouncementToolbar}/>{!!cloud.status.conflicts?.length&&<button className="icon-button sync-conflicts-button" disabled={!!busy} title={t('处理同步冲突')} aria-label={t('处理同步冲突')} onClick={()=>openSyncConflict(cloud.status.conflicts?.[0]?.localId)}><CircleAlert size={18}/><span className="sync-conflict-count">{new Set(cloud.status.conflicts.map(c=>c.localId)).size}</span></button>}{mcpPending&&<button className="icon-button mcp-pending" title={t("MCP 有待确认操作")} aria-label={t("MCP 有待确认操作")} onClick={()=>setPage('automation')}><PlugZap size={18}/></button>}<button className="icon-button" title={t("使用指南")} aria-label={t("使用指南")} onClick={() => setModal({ kind: 'help' })}><CircleHelp size={18} /></button><WindowControls onError={message => notify(message)} /></div></header>
      <div className="main-content">
        <AnnouncementCenter announcements={siteCatalog.announcements??[]} toolbarTarget={announcementToolbar}/>
        {page!=='profiles'&&<div className="page-heading"><div><h1>{t(pageTitles[page])}</h1>{['computer','overview'].includes(page)&&<p>{scan ? <>{t("上次检测：")}{timeLabel(scan.checkedAt)}</> : t("尚无检测结果，点击右上角“重新检测”开始。")}</p>}</div>{!['settings','account','devices','sharing','automation','sync','appProxy'].includes(page)&&<button className="button secondary" style={{visibility:page==='code'?'hidden':undefined}} disabled={!!busy} onClick={() => page==='history' ? perform('正在读取修复记录',async()=>{await refreshHistory();notify('修复记录已更新');}) : perform('正在重新检测', async () => { await refresh(); })}><RefreshCw size={15} className={(page==='history'?historyLoading.has(environmentScopeKey(currentScope.current)):busy?.includes('检测')) ? 'spin' : ''} />{t(page==='history' ? historyLoading.has(environmentScopeKey(currentScope.current))?'正在读取修复记录':'刷新记录' : busy?.includes('检测') ? '正在检测' : '重新检测')}</button>}</div>}

        <AppUpdates expanded={page === 'settings'} busy={!!busy} onBusyChange={value=>setBusy(value?'正在更新应用':null)} />
        {(page==='sync'||(page==='account'||page==='devices')&&cloud.status.account)&&<AccountDialog key={cloud.status.account?.id||'signed-out'} inline cloud={{...cloud,panel:page as 'sync'|'account'|'devices'}} focusProfileId={syncTarget}/>}
        {page==='profiles'&&<BrowserProfiles onSelect={(p,history)=>void selectProfile(p,history)} onShare={setShareProfile} onManageSharing={()=>setPage('sharing')} syncConflicts={cloud.status.conflicts} onResolveSync={openSyncConflict} onSyncStatusChange={()=>void cloud.reload()} onProfileReset={handleProfileReset} busy={!!busy||sharing.busy} onBusyChange={value=>setBusy(value?'正在管理浏览器副本':null)}/>}
        {page==='sharing'&&<SharingWorkspace sharing={sharing} cloud={cloud}/>}
        {page==='appProxy'&&<ApplicationProxy/>}
        <ShareProfileDialog profile={shareProfile} sharing={sharing} cloud={cloud} onClose={()=>setShareProfile(null)}/>
        {activeProfile&&['overview','history'].includes(page)&&<div className="active-profile-bar"><div className={page==='overview'?'overview-profile-identity':undefined}>{page==='overview'?<label className="overview-profile-picker"><span>{t("当前副本")}</span><SelectMenu menuMinWidth={320} noWrap label={t("切换浏览器副本")} disabled={!!busy} value={activeProfile.id} onOpen={()=>void loadProfileOptions().catch(e=>notify(String(e),true))} onChange={value=>{const p=profileOptions.find(p=>p.id===value);if(p&&p.id!==activeProfile.id)void selectProfile(p);}} options={(!profileOptions.some(p=>p.id===activeProfile.id)?[activeProfile,...profileOptions]:profileOptions).map(p=>({value:p.id,label:`${p.name} · ${browserNames[p.browser]}`,icon:<BrowserIcon browser={p.browser}/>}))} /></label>:<strong>{activeProfile.name}</strong>}<span>{t(browserNames[browser])} · {t(activeProfile.proxy.mode==='system'?'系统代理':activeProfile.proxy.mode==='direct'?'直连':activeProfile.proxy.host+':'+activeProfile.proxy.port)}</span></div><button disabled={!!busy} className={page==='overview'?'selected':''} onClick={()=>setPage('overview')}>{t("环境检测")}</button><button disabled={!!busy} className={page==='history'?'selected':''} onClick={()=>void navigate('history')}>{t("修复记录")}</button><button disabled={!!busy} onClick={()=>setPage('profiles')}>{t("返回列表")}</button></div>}
        {page==='computer'&&<div className="computer-sections"><div className="notice"><Info size={18}/><p>{t("这里的修改会影响电脑的所有应用。更改时区或卸载用户字体前，需退出全部专用浏览器副本。")}</p></div><section className="check-table">{scan ? checks.map(checkRow) : <div className="empty-state">{busy?.includes('检测')?<LoaderCircle className="spin" size={28}/>:<Monitor size={28}/>}<strong>{t(busy?.includes('检测')?'正在检测':'尚未检测')}</strong><p>{t("点击右上角“重新检测”获取当前环境。")}</p></div>}</section><div className="notice"><Clock3 size={18}/><p>{t("时区只影响本地时间显示，不会校准电脑时钟。若网页提示 Incorrect device time，请在系统日期与时间中启用自动设置时间并立即同步，然后重启副本复检。")}</p></div><section className="settings-card"><h3>{t("用户字体")}</h3><p>{t("查看可卸载的用户字体，修改前备份，支持从电脑级修复记录恢复。")}</p><div className="settings-actions"><button className="button secondary" disabled={!!busy} onClick={()=>void fontDialog()}>{t("管理用户字体")}</button><button className="button secondary" disabled={!!busy} onClick={()=>void navigate('history')}>{t("查看电脑级修复记录")}</button></div></section></div>}
        {page === 'overview' && browser === 'firefox' && scan && !scan.browserAvailable && <FirefoxSetup busy={!!busy}
          onDownload={() => void perform('正在打开 Firefox 官方下载页', openFirefoxDownload)}
          onRefresh={() => void perform('正在重新检测 Firefox', () => refresh())} />}
        {page === 'overview' && <>
          <div className="workspace-actions overview-browser-actions"><button className="button primary" disabled={!!busy || !(scan?.browserAvailable??activeProfile?.browserAvailable)} onClick={() => void openBrowser('verify')}><Globe2 size={16} />{t("打开浏览器")}<ArrowUpRight size={15} /></button><BrowserSiteShortcuts disabled={!!busy || !(scan?.browserAvailable??activeProfile?.browserAvailable)} onOpen={(id,revision)=>void openBrowser(`site:${id}@${revision}`)} /></div>
          {!(scan?.browserAvailable??activeProfile?.browserAvailable) && scan && browser !== 'firefox' && <div className="notice warning-notice"><Info size={17} /><p>{t("没有找到")}{t(browserNames[browser])}{t("。请先安装，或切换到其他浏览器副本，然后重新检测。")}</p></div>}
          <div className="hero-grid">
            <section className="hero-panel"><div className="hero-copy"><span className="eyebrow"><span />ENVIRONMENT CHECK</span><h2>{t(!scan ? busy?.includes('检测')?'正在检测':'尚未检测' : warnings.length ? `${warnings.length} 项建议调整` : '未发现建议调整项')}</h2><p>{t(!scan ? busy?.includes('检测')?'读取本地配置并检查网络连接，请稍候。':'点击右上角“重新检测”获取当前环境。' : '已配置的项目可在专用浏览器中继续复检。')}</p><button className="button primary" disabled={!!busy || !scan || !suggested.length} onClick={bulkRepair}><Sparkles size={16} />{t(!scan?'一键修复':suggested.length ? `一键修复 ${suggested.length} 项` : '自动修复项已配置')}<ArrowRight size={15} /></button><span className="hero-note"><History size={12} />{t("自动备份原值，随时可以撤销")}</span></div><Orbit count={warnings.length} total={checks.length||checkCount} /></section>
            <section className="environment-card"><div className="card-topline"><span>{t("你的使用环境")}</span><span className="tiny-label">LOCAL</span></div><div className="environment-os"><span className="os-symbol"><Monitor size={25} strokeWidth={1.4} /></span><div><strong>{t(scan?.platform || '尚未检测')}</strong><small>{t("桌面环境 · 本地配置")}</small></div></div><div className="environment-line"><span>{t("专用浏览器")}</span><div><SelectMenu label={t("选择专用浏览器")} value={browser} onChange={value=>void switchBrowser(value as BrowserId)} disabled={!!busy||Boolean(activeProfile)} options={[{value:"chrome",label:"Google Chrome",icon:<BrowserIcon browser="chrome"/>},{value:"edge",label:"Microsoft Edge",icon:<BrowserIcon browser="edge"/>},{value:"firefox",label:t("Firefox · 保留电脑字体"),icon:<BrowserIcon browser="firefox"/>}]} /></div></div><div className="environment-line"><span>{t("Cloudflare 检测出口")}</span><strong className="ip-label">{t(scan?.ip || '尚未获取')}{t(scan?.location && <span className="country">{t(scan.location)}</span>)}</strong></div><div className="environment-footer"><LockKeyhole size={12} />{t("专用配置与日常浏览器相互独立")}</div></section>
          </div>
          <div className="summary-bar"><span><span className="summary-dot amber" /><strong>{warnings.length}</strong> {t("项建议调整")}</span><span><span className="summary-dot green" /><strong>{healthy}</strong> {t("项检测正常")}</span><span><span className="summary-dot blue" /><strong>{configured}</strong> {t("项已配置")}</span><span><span className="summary-dot gray" /><strong>{manual}</strong> {t("项需确认")}</span><span className="last-check"><Clock3 size={12} />{t(scan ? `${timeLabel(scan.checkedAt)} 检测` : '尚未检测')}</span></div>
          <div className="notice signal-explainer"><Info size={17} /><p>{t("第三方网页的加权分数不是 Claude 官方账号风险评分。语言、时区可调整；GPU、屏幕和平台风格是正常设备特征。请通过下方专用窗口复检，日常浏览器不会继承这些设置。")}</p></div><section className="browser-verification-bar"><div><strong>{t("网页实际结果，单独复检")}</strong><p>{t(reports[reportKey] ? `已导入 ${timeLabel(reports[reportKey]!.capturedAt)} 报告 · ${reports[reportKey]!.browser ? '专用页面标记' : '无专用标记，请确认窗口'}；设置变更后请重新采集。` : '11 项网页信号单独采集；网页检测无法验证 Claude Code 进程。')}</p></div><button className="button secondary small-button" disabled={!!busy || !(scan?.browserAvailable??activeProfile?.browserAvailable)} onClick={() => void openBrowser('diagnostics')}>{t("打开本地复检")}<ArrowUpRight size={13} /></button><button className="button secondary small-button" disabled={!!busy || !scan} onClick={() => { setReportInput(''); setModal({kind:'report'}); }}>{t("导入浏览器复检")}</button>{reports[reportKey] && <button className="row-detail" onClick={() => setReports(prev => ({...prev, [reportKey]:null}))}>{t("清除报告")}</button>}</section><section className="checks-section"><div className="section-heading"><h3>{t("检测项目")}<span>{t(checks.length.toString().padStart(2, '0'))}</span></h3><label className="pending-toggle"><input type="checkbox" checked={onlyPending} onChange={e => setOnlyPending(e.target.checked)} /><span className="switch" />{t("只看待处理")}{onlyPending && <span className="tiny-label">{pendingCount}</span>}</label></div><div className="filter-tabs" role="tablist" aria-label={t("检测类别")}>{[['all', '全部项目'], ['network', '网络连接'], ['privacy', '隐私保护'], ['device', '设备环境']].map(([id, title]) => <button role="tab" aria-selected={group === id} key={id} className={group === id ? 'selected' : ''} onClick={() => setGroup(id)}>{t(title)}{id === 'all' && <span>{checks.length}</span>}</button>)}</div><div className="check-table"><div className="table-head"><span>{t("检测项目")}</span><span>{t("当前状态")}</span><span>{t("检测结果")}</span><span>{t("操作")}</span></div>{!scan ? <div className="loading-state">{busy?.includes('检测')?<LoaderCircle className="spin" size={28} />:<Monitor size={28}/>}<strong>{t(busy?.includes('检测')?'正在检测':'尚未检测')}</strong><p>{t(busy?.includes('检测')?'网络检测可能需要约 30 秒。检测只读取本地配置。':'点击右上角“重新检测”获取当前环境。')}</p></div> : displayed.length ? displayed.map(checkRow) : <div className="empty-state"><CheckCheck size={29} /><strong>{t("这个分类没有待处理项目")}</strong><p>{t("可以查看全部项目，或到浏览器里复检。")}</p><button onClick={() => { setOnlyPending(false); setGroup('all'); }}>{t("查看全部项目")}<ArrowRight size={14} /></button></div>}</div></section>
          <details className="firefox-help overview-browser-help"><summary>{t("浏览器说明与配置位置")}</summary><div className="overview-browser-help-content">
          <div className="notice"><Info size={17} /><p>{t("使用独立配置目录打开")}{t(browserNames[browser])}{t("，语言与隐私设置仅在这里生效。首次使用需要重新登录 Claude。")}</p></div>
          {browser === 'firefox' && <div className="notice firefox-notice"><Type size={19} /><div><strong>{t("保留电脑字体，为专用 Firefox 单独设置")}</strong><p>{t("主动选择“限制字体”后应用。设置只影响此专用环境；常见拉丁字体和 Emoji 保留，部分中文显示可能变化。此选项不自动加入一键修复。")}</p><p>{t("Firefox 完全退出后才能修改或从工具重新打开；已打开时，可在当前专用窗口继续访问网站。")}{t(!(scan?.browserAvailable??activeProfile?.browserAvailable) && scan && '请先安装 Firefox。')}</p><button className="button secondary small-button" disabled={!!busy || !(scan?.browserAvailable??activeProfile?.browserAvailable)} onClick={() => void fontDialog()}>{t("设置字体可见性")}<ArrowRight size={14} /></button></div></div>}
          {browser === 'firefox' && <details className="firefox-help"><summary>{t("Firefox 启动或安全连接失败？")}</summary><p><strong>{t("配置文件缺失：")}</strong>{t("请正常退出专用 Firefox 后重试。本工具会创建并检查专用目录；若仍失败，请反馈下方的配置保存位置和错误截图，无需删除日常配置。")}</p><p><strong>{t("访问 home.firefoxchina.cn 失败：")}</strong>{t("这可能来自中国版主页或安装包自带设置，建议使用上方官方渠道的安装包。专用环境启动主页设为空白，并直接打开指定页面。")}</p><p><code>PR_END_OF_FILE_ERROR</code> {t("也可能由代理、VPN、加密 DNS 或安全软件引起，不能仅凭错误码判断安装包版本。先尝试本地环境复检，再打开 Claude；若仅在修复 DNS 后出现问题，可在修复记录中恢复“DNS 隐私”后重试。")}</p><button className="button secondary small-button" disabled={!!busy} onClick={() => void perform('正在打开 Firefox 官方下载页', openFirefoxDownload)}>{t("Firefox 官方下载")}<ArrowUpRight size={14} /></button></details>}
          <p>{t("“已配置”表示设置已写入。浏览器策略或网络代理可能影响实际效果，请通过网页复检。")}</p>
          <p>{t("修复前保存原值。恢复时仅修改对应字段，保留后来产生的登录信息和其他偏好。")}</p>
          <div className="path-card"><span>{t("配置保存位置")}</span><code>{t(scan?.profilePath || '尚未检测')}</code><LockKeyhole size={14} /></div>
          </div></details>
        </>}

        {page === 'code' && <TerminalWorkspace busy={!!busy} onBusyChange={setBusy} onNotify={notify} />}

        {page === 'history' && <>
          <div className="history-intro"><span className="large-feature-icon"><History size={29} strokeWidth={1.5} /></span><div><p>{t("原值保存在本机。撤销时会检查设置是否又被其他程序修改。")}</p></div><span className="tiny-label">{history.length} {t("条记录")}</span></div>
          {!history.length&&historyLoading.has(environmentScopeKey(currentScope.current)) ? <div className="loading-state" role="status"><LoaderCircle className="spin" size={28}/><strong>{t("正在读取修复记录")}</strong></div> : !history.length ? <div className="history-empty"><span className="empty-illustration"><History size={43} strokeWidth={1.1} /></span><h2>{t("还没有修复记录")}</h2><p>{t("完成一次修复后，原值和处理结果会出现在这里。")}</p><button className="button secondary" onClick={() => void navigate('overview')}>{t("返回环境概览")}<ArrowRight size={15} /></button></div> : <div className="history-list">{history.map(r => <article className={`history-record ${r.status === 'undone' ? 'undone' : ''}`} key={`${r.profileId||'computer'}-${r.id}`}><span className="check-icon"><History size={18} /></span><div className="record-body"><div><strong>{t(definitions[r.itemId].title)}</strong><span className={`record-status ${r.status}`}>{t(({ applied: '已应用', undone: '已撤销', failed: '未完成', pending: '待核验' })[r.status])}</span></div><p>{t(r.message)}</p><small>{t(timeLabel(r.createdAt))}<span>·</span>{t(!r.profileId ? '电脑环境' : `${browserNames[r.browser]} · 副本 ${r.profileId.startsWith('legacy-')?'默认':r.profileId.slice(0,8)}`)}{t(!native && ' · 演示记录')}</small><details><summary>{t("查看备份内容")}<ChevronDown size={12} /></summary>{r.changes.map((c, i) => <div className="backup-detail" key={i}><code>{t(c.pointer || c.target)}</code><span>{t("原值：")}{formatBackup(c.before)}</span><span>{t("目标：")}{formatBackup(c.after)}</span></div>)}</details></div>{r.status !== 'undone' && <button className="button secondary small-button" disabled={!!busy} onClick={() => setModal({ kind: 'undo', record: r })}><RotateCcw size={13} />{t("恢复原值")}</button>}</article>)}</div>}
        </>}

        <ThemeControl visible={page==='settings'} />
        {page==='automation'&&<McpWorkspace/>}
        {page === 'settings' && <>
          <AutostartSettings/>
          <CloudSettings cloud={cloud}/>
          <section className="settings-card about-card"><div className="brand"><Mark small /><span>NodeCloak<small>{t("版本")}{t(version)} · Tauri 2</small></span></div><p>{t("管理独立浏览器副本，为每个副本配置代理、语言和隐私设置，并检查网络连接与浏览器环境。")}</p></section>
        </>}
      </div>
    </main>
    <AccountDialog cloud={{...cloud,panel:cloud.panel==='sync'?null:cloud.panel}}/>
    {t(busy && scan && <div className="operation-indicator" role="status"><LoaderCircle size={14} className="spin" />{t(busy)}</div>)}
    {toast && <div className={`toast ${toast.error ? 'error' : ''}`} role={toast.error ? 'alert' : 'status'}>{toast.error ? <Info size={18} /> : <Check size={18} />}<span>{t(toast.text)}</span>{toast.syncRecovery&&<button type="button" className="button secondary compact" onClick={()=>openSyncConflict(toast.syncProfileId)}><Settings2 size={14}/>{t('处理同步冲突')}</button>}<button aria-label={t("关闭通知")} onClick={() => setToast(null)}><X size={15} /></button></div>}
    <dialog ref={quitDialog} className="app-dialog" onCancel={e=>{e.preventDefault();if(!busy)setQuitting(false);}}><h2>{t("退出 NodeCloak？")}</h2><p className="modal-subtitle">{t("真正退出将先关闭运行中的浏览器副本和本地代理。请保存各窗口中未完成的输入。")}</p>{t(quitError&&<p role="alert" className="profile-error">{t(quitError)}</p>)}<div className="modal-actions"><button className="button secondary" disabled={!!busy} onClick={()=>setQuitting(false)}>{t("继续运行")}</button><button className="button primary" disabled={!!busy} onClick={()=>void quit()}>{busy?<LoaderCircle className="spin" size={15}/>:null}{t("关闭副本并退出")}</button></div></dialog>
    <dialog ref={dialogRef} className="modal" onCancel={e => { if (busy) e.preventDefault(); else setModal(null); }} onClick={e => { if (e.target === e.currentTarget && !busy) setModal(null); }} aria-labelledby="modal-title"><div className="modal-content"><button className="modal-close icon-button" aria-label={t("关闭面板")} disabled={!!busy} onClick={() => setModal(null)}><X size={19} /></button>
      {modal?.kind === 'fonts' && <FontReview key="font-review" catalog={modal.catalog} initialIds={modal.initialIds} busy={!!busy} onClose={() => setModal(null)} onSubmit={(ids,consent) => void applyFonts(ids,consent)}/>}
      {modal?.kind === 'font-result' && <FontResult outcomes={modal.outcomes} busy={!!busy} onHistory={() => {setModal(null);void navigate('history');}} onRetry={ids => void fontDialog(ids)}/>}
      {modal?.kind === 'repair' && <><span className="modal-feature"><Sparkles size={24} /></span><span className="eyebrow">A LITTLE TUNE-UP</span><h2 id="modal-title">{t("准备修复这些设置")}</h2><p className="modal-subtitle">{t("应用前自动保存原值。请选择要处理的项目。")}</p>{!native && <div className="notice demo-notice"><Info size={15} /><p>{t("当前为交互演示，以下操作只更新演示数据。")}</p></div>}<div className="repair-options">{[...new Set([...modal.ids, ...(browser === 'firefox' ? ['fonts' as CheckId] : []), ...(!modal.ids.includes('fonts') && checks.some(c => ['timezone','offset'].includes(c.id) && c.status === 'warning') ? ['timezone' as CheckId] : [])])].map(id => <label key={id} className={`repair-option ${selected.includes(id) ? 'checked' : ''}`}><input type="checkbox" checked={selected.includes(id)} disabled={!!busy} onChange={e => { setSelected(prev => e.target.checked ? [...prev, id] : prev.filter(x => x !== id)); if (id === 'timezone') setConsentTimezone(false); if(id === 'fonts') setConsentFonts(false); }} /><span className="custom-checkbox">{selected.includes(id) && <Check size={12} />}</span><span><strong>{t(definitions[id].title)}{id === 'timezone' && <em>{t("影响整个系统")}</em>}</strong><small>{t(id === 'timezone' ? timezoneLabel : id === 'webrtc' && browser === 'firefox' ? '关闭专用 Firefox 的 WebRTC · 可能影响通话' : definitions[id].target)}</small></span></label>)}</div>{selected.includes('fonts') && <label className="timezone-consent"><input type="checkbox" checked={consentFonts} onChange={e => setConsentFonts(e.target.checked)} disabled={!!busy} /><span>{t("我确认只限制专用 Firefox 的系统字体可见性，保留电脑中文字体。部分中文显示和网页排版可能变化；网页下载字体仍可使用。需要退出并重新打开专用 Firefox 复检，可从修复记录恢复。")}</span></label>}{selected.includes('timezone') && <TimezonePicker target={timezoneTarget} custom={customTimezone} options={timezoneCatalog} loading={timezoneLoading} error={timezoneError} platform={scan?.platform || ''} disabled={!!busy} onTarget={target => { setTimezoneTarget(target); setConsentTimezone(false); }} onCustom={zone => { setCustomTimezone(zone); setConsentTimezone(false); }} onReload={() => void loadTimezoneCatalog()} />}{selected.includes('timezone') && <label className="timezone-consent"><input type="checkbox" checked={consentTimezone} onChange={e => setConsentTimezone(e.target.checked)} disabled={!!busy} /><span>{t("我确认将整个系统时区改为")}{t(timezoneLabel)}{t("，已了解上述时间变化。这会影响所有应用；Windows 或 macOS 可能请求系统管理员授权。")}</span></label>}<div className="notice"><History size={16} /><p>{t("修复浏览器设置前，请关闭本工具打开的专用窗口。WebRTC 限制可能影响音视频通话；严格加密 DNS 不可达时网站可能无法访问。")}</p></div><div className="modal-actions"><button className="button secondary" disabled={!!busy} onClick={() => setModal(null)}>{t("暂不修复")}</button><button className="button primary" disabled={!!busy || !selected.length || (selected.includes('timezone') && (!consentTimezone || !customTimezoneValid)) || (selected.includes('fonts') && !consentFonts)} onClick={() => void applyRepair()}>{busy ? <LoaderCircle className="spin" size={15} /> : <Sparkles size={15} />}{t(busy ? '正在备份与修复' : `${native ? '应用' : '演示'} ${selected.length} 项修复`)}</button></div></>}
      {modal?.kind === 'detail' && (() => { const c = checks.find(c => c.id === modal.id); const def = definitions[modal.id]; const Icon = icons[modal.id]; return <><span className="modal-feature"><Icon size={25} /></span><h2 id="modal-title">{t(def.title)}</h2><p className="modal-subtitle">{t(def.description)}</p>{c && <div className="detail-result"><Status status={c.status} id={c.id} /><strong>{t(c.value)}</strong><p>{t(c.detail)}</p></div>}<h3 className="detail-heading">{t("你可以这样处理")}</h3><ol className="advice-list">{(modal.id === 'fonts' && browser === 'firefox' ? def.advice.slice(0,1) : def.advice).map(text => <li key={text}>{t(text)}</li>)}</ol><div className="modal-actions"><button className="button secondary" onClick={() => setModal(null)}>{t("知道了")}</button>{modal.id === 'clock' ? <button className="button primary" disabled={!!busy} onClick={() => void perform('正在打开系统设置', openTimezoneSettings)}>{t("打开系统日期与时间")}<ArrowUpRight size={14} /></button> : modal.id === 'fonts' ? <button className="button primary" disabled={!!busy} onClick={() => void fontDialog()}>{t(browser === 'firefox' ? '设置字体可见性' : '管理可卸载字体')}<ArrowRight size={14} /></button> : c?.fixable && (c.status === 'warning' || c.id === 'timezone' || c.id === 'offset') ? <button className="button primary" disabled={!!busy} onClick={() => repairDialog([c.id])}>{t(['timezone','offset'].includes(c.id) ? '设置时区' : '修复这一项')}<ArrowRight size={14} /></button> : <button className="button primary" disabled={!!busy || !(scan?.browserAvailable??activeProfile?.browserAvailable)} onClick={() => void openBrowser('verify')}>{t("打开网页复检")}<ArrowUpRight size={14} /></button>}</div></>; })()}
      {modal?.kind === 'undo' && <><span className="modal-feature"><RotateCcw size={25} /></span><h2 id="modal-title">{t("恢复修复前的设置？")}</h2><p className="modal-subtitle">{t("将恢复「")}{t(definitions[modal.record.itemId].title)}{t("」在")}{t(timeLabel(modal.record.createdAt))} {t("修复前的原值。")}</p><div className="notice"><Info size={17} /><p>{t("关闭专用浏览器后再恢复。如果这个设置后来被其他程序修改，工具会保留新设置并提示你。登录信息和其他偏好会保留。")}</p></div><div className="modal-actions"><button className="button secondary" disabled={!!busy} onClick={() => setModal(null)}>{t("取消")}</button><button className="button primary" disabled={!!busy} onClick={() => perform('正在恢复原值', async () => { await undoRepair(modal.record.id,modal.record.profileId); invalidateHistory(); setReports({}); setModal(null); await refresh(); notify('原值已恢复，可在修复记录中查看结果。'); })}>{busy ? <LoaderCircle size={15} className="spin" /> : <RotateCcw size={15} />}{t("恢复原值")}</button></div></>}
      {modal?.kind === 'blocked' && <><span className="modal-feature partial"><Info size={26} /></span><span className="eyebrow">BEFORE WE CONTINUE</span><h2 id="modal-title">{t("先退出专用浏览器")}</h2><p className="modal-subtitle">{t("本次修复尚未开始，选择的")}{modal.request.ids.length} {t("项设置已保留。")}</p><div className="notice warning-notice"><Info size={17} /><p>{t(modal.reason)}</p></div><ol className="advice-list repair-preparation"><li>{t("保存专用窗口中未完成的输入。")}</li><li>{t("从该窗口的浏览器菜单选择“退出”。仅关掉标签页可能保留后台进程。")}</li><li>{t("日常浏览器可以继续使用。退出后，点击下方按钮重新检查。")}</li></ol><div className="modal-actions"><button className="button secondary" disabled={!!busy} onClick={() => setModal(null)}>{t("稍后处理")}</button><button className="button primary" disabled={!!busy} onClick={() => void applyRepair(modal.request)}><RefreshCw size={15} className={busy ? 'spin' : ''} />{t("我已退出，检查并继续")}</button></div></>}
      {modal?.kind === 'result' && (() => { const summary = summarizeOutcomes(modal.outcomes); return <><span className={'modal-feature ' + summary.state}>{summary.state === 'success' ? <CheckCheck size={26} /> : <CircleAlert size={26} />}</span><span className="eyebrow">{t(summary.state === 'success' ? 'SETTINGS UPDATED' : 'NEEDS ATTENTION')}</span><h2 id="modal-title">{t(summary.title)}</h2><p className="modal-subtitle">{t(native ? '完成表示设置已写入并验证；实际浏览器效果仍需复检。' : '以下为交互演示结果，电脑设置没有改变。')}{t(summary.failedIds.length > 0 && ' 请先处理未完成项目，再打开浏览器复检。')}</p><div className="result-list">{modal.outcomes.map(o => <div key={o.id} className={o.success ? 'completed' : 'incomplete'}>{o.success ? <Check className="success-icon" size={18} /> : <Info className="warning-icon" size={18} />}<span><strong>{t(definitions[o.id].title)}<em className="result-item-status">{t(o.success ? '已完成' : '未完成')}</em></strong><small>{t(o.message)}</small></span></div>)}</div>{summary.failedIds.includes('timezone') && scan?.platform === 'Windows' && <button className="timezone-settings-link" disabled={!!busy} onClick={() => void perform('正在打开系统设置', openTimezoneSettings)}>{t("打开系统日期与时间")}<ArrowUpRight size={14} /></button>}<div className="modal-actions result-actions"><button className="button secondary" disabled={!!busy} onClick={() => { setModal(null); void navigate('history'); }}>{t("查看修复记录")}</button>{summary.failedIds.length > 0 ? <button className="button primary" disabled={!!busy} onClick={() => repairDialog(summary.failedIds, modal.request.timezoneTarget, modal.request.customTimezone)}><RefreshCw size={15} />{t("重试")}{summary.failedIds.length} {t("个未完成项目")}</button> : <button className="button primary" disabled={!!busy || !(scan?.browserAvailable??activeProfile?.browserAvailable)} onClick={() => void openBrowser('verify')}>{t("打开浏览器")}<ArrowUpRight size={15} /></button>}</div></>; })()}
      {modal?.kind === 'report' && <><span className="modal-feature"><Activity size={25} /></span><h2 id="modal-title">{t("导入浏览器复检")}</h2><p className="modal-subtitle">{t("请在本工具打开的")}{t(browserNames[browser])} {t("专用窗口中打开本地复检页，复制完整 JSON 报告后粘贴到这里。")}</p><textarea className="report-input" aria-label={t("浏览器复检 JSON 报告")} placeholder={t("粘贴本地复检页的完整报告")} value={reportInput} onChange={e => setReportInput(e.target.value)} spellCheck={false} /><div className="notice"><Info size={16} /><p>{t("报告保存在当前应用会话，不上传服务器。这里只显示采集时的 11 项浏览器信号，不给账户安全评分。导入旧版报告时，新增项目保持未确认；字体宽度和 UA 推断都有限制。")}</p></div><div className="modal-actions"><button className="button secondary" onClick={() => setModal(null)}>{t("取消")}</button><button className="button primary" disabled={!reportInput.trim()} onClick={importReport}>{t("导入结果")}<ArrowRight size={14} /></button></div></>}{modal?.kind === 'help' && <><span className="modal-feature"><BookOpen size={25} /></span><h2 id="modal-title">{t("三步，准备你的环境。")}</h2><p className="modal-subtitle">{t("第一次使用，从这里开始。")}</p><div className="guide-steps">{[['01', '先检测，看清现状', '本机检测读取专用浏览器设置，并请求 Claude 和 Cloudflare 检查连接。网页端项目仍需在实际浏览器中复检。'], ['02', '按需修复，自动备份', '一键修复默认调整专用浏览器与 Claude Code 启动器。系统时区与 Firefox 字体限制需要单独确认。所有调整都可以从记录恢复。'], ['03', '打开专用环境，继续使用', '网页版请从环境概览页启动；Claude Code、Codex CLI、Gemini CLI 或其他工具请从终端页启动。在其他浏览器或终端中使用时，独立设置不会自动生效。']].map(([number, title, text]) => <div key={number}><span>{t(number)}</span><div><strong>{t(title)}</strong><p>{t(text)}</p></div></div>)}</div><div className="modal-actions"><button className="button primary" onClick={() => setModal(null)}>{t("了解了")}<ArrowRight size={15} /></button></div></>}
    </div></dialog>
  </div>;
}
function formatBackup(value: unknown) {
  if (value === null || value === undefined) return '未设置（恢复时移除此字段）';
  const text = typeof value === 'string' ? value : JSON.stringify(value);
  return text.length > 240 ? `${text.slice(0, 240)}…` : text;
}


