import {t,getDateLocale} from './i18n';
import { useEffect, useRef, useState } from 'react';
import { Check, CircleAlert, Download, LoaderCircle, RefreshCw, X } from 'lucide-react';
import { checkForUpdates, installAvailableUpdate, restartAfterUpdate, openDownloadPage, native, type UpdateCheck, type UpdateProgress } from './bridge';
import {listProfiles,prepareApplicationUpdate} from './browser-profiles';
import { version } from '../package.json';

const PREFERENCE = 'claudedone.auto-check-updates';
const SIX_HOURS = 6 * 60 * 60 * 1000;

export default function AppUpdates({ expanded, busy=false, onBusyChange }: { expanded: boolean; busy?: boolean; onBusyChange?:(value:boolean)=>void }) {
  const [automatic, setAutomatic] = useState(() => {
    try { return localStorage.getItem(PREFERENCE) !== 'off'; } catch { return true; }
  });
  const [result, setResult] = useState<UpdateCheck | null>(null);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState('');
  const [dismissed, setDismissed] = useState('');
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [updating, setUpdating] = useState(false);
  const [updateConfirmation,setUpdateConfirmation]=useState(false);
  const confirmDialog=useRef<HTMLDialogElement>(null);
  useEffect(()=>{if(updateConfirmation)confirmDialog.current?.showModal();else confirmDialog.current?.close();},[updateConfirmation]);
  const installing = useRef(false);
  const inFlight = useRef(false);
  const initialCheck = useRef(false);
  const mounted = useRef(true);

  async function check() {
    if (inFlight.current || installing.current || busy) return;
    inFlight.current = true;
    setChecking(true); setError(''); setProgress(null);
    try {
      const next = await checkForUpdates();
      if (mounted.current) setResult(next);
    } catch (failure) {
      if (mounted.current) setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      inFlight.current = false;
      if (mounted.current) setChecking(false);
    }
  }
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    if (!automatic || busy) return;
    if (!initialCheck.current) { initialCheck.current = true; void check(); }
    const timer = window.setInterval(() => void check(), SIX_HOURS);
    return () => clearInterval(timer);
  }, [automatic,busy]);

  function toggleAutomatic(value: boolean) {
    setAutomatic(value);
    try { localStorage.setItem(PREFERENCE, value ? 'on' : 'off'); } catch { /* Still applies to this session. */ }
    if (value) void check();
  }
  async function install(confirmed=false) {
    if(installing.current || busy || checking) return;
    if(!confirmed){try{if((await listProfiles()).some(p=>p.running)){setUpdateConfirmation(true);return;}}catch(e){setError(String(e));return;}}
    setUpdateConfirmation(false);installing.current=true; setUpdating(true); setError(''); onBusyChange?.(true);
    let installed=false;
    try {
      await prepareApplicationUpdate();
      await installAvailableUpdate(next=>{if(mounted.current) setProgress(next);});
      installed=true;
      if(native) await restartAfterUpdate();
    } catch(failure) {
      if(mounted.current) {
        setError(installed ? '更新已安装，自动重启未完成。请点击“重启应用”或退出后重新打开。' : `更新未完成：${failure instanceof Error ? failure.message : String(failure)}。可重试更新。`);
        if(!installed) setProgress(null);
      }
    } finally {
      installing.current=false;
      if(mounted.current) setUpdating(false);
      onBusyChange?.(false);
    }
  }
  async function restart() {
    try { await restartAfterUpdate(); } catch { setError('请退出应用后重新打开，已安装的更新会生效。'); }
  }
  async function downloadManually() {
    try { await openDownloadPage(); } catch(failure) { setError(failure instanceof Error ? failure.message : String(failure)); }
  }

  const updateVisible = result?.available && (expanded || dismissed !== result.latestVersion);
  const updatingVisible=updating || progress?.phase==='ready';
  if (!expanded && !updateVisible && !updatingVisible) return null;
  const percent=progress?.total ? Math.min(100,Math.floor(progress.downloaded/progress.total*100)) : null;
  const progressText=progress?.phase==='ready' ? native ? '更新已安装，正在重启应用…' : '演示更新已完成，电脑没有改变。' : progress?.phase==='installing' ? '正在安装更新，请勿关闭应用…' : progress?.phase==='verifying' ? '下载完成，正在校验更新签名…' : `正在下载更新${percent!==null ? ` · ${percent}%` : ''}…`;
  return <section className={expanded ? 'settings-card app-updates' : 'update-banner'} aria-label={t("应用更新")}>
    {expanded && <>
      <div className="update-heading"><div><h3>{t("应用更新")}</h3><p>{t("当前版本")}{t(version)}{t(!native && ' · 演示模式')}</p></div><span className="settings-tag neutral">{t("稳定版")}</span></div>
      {!result?.manualDownload && <div className="settings-line"><div><strong>{t("自动检查更新")}</strong></div><label className="pending-toggle"><input type="checkbox" role="switch" checked={automatic} disabled={updating} onChange={event => toggleAutomatic(event.target.checked)} aria-label={t("自动检查更新")} /><span className="switch" /></label></div>}
      <div className="update-status" role="status" aria-live="polite">
        {checking ? <><LoaderCircle size={16} className="spin" /><span>{t("正在检查新版本…")}</span></> : error ? <><CircleAlert size={16} /><span>{t(error)}</span></> : result ? <><Check size={16} /><span>{t(result.manualDownload ? '当前安装方式使用手动更新' : result.available ? `发现新版本 ${result.latestVersion}` : result.supported ? '当前已是最新版本' : '此平台暂无可下载的官方安装包')}</span></> : <span>{t("尚未检查更新")}</span>}
      </div>
      {result && !result.manualDownload && <p className="update-last-check">{t("上次检查成功：")}{t(new Date(result.checkedAt).toLocaleString(getDateLocale()))}</p>}
    </>}
    {(updateVisible || updatingVisible) && result && <div className="update-release" role="status">
      <div><strong><Download size={16} />{t("发现新版本")}{t(result.latestVersion)}{t(!native && '（演示）')}</strong><ul>{result.notes.map((note, index) => <li key={index}>{t(note)}</li>)}</ul>{result.portable && <p className="update-open-error">{t("当前为便携版，更新将通过安装器安装并启动新版；原便携文件保留。后续请使用安装版。")}</p>}</div>
      <div className="update-release-actions">{progress?.phase==='ready' ? native && <button className="button primary" onClick={() => void restart()}><RefreshCw size={14} />{t("重启应用")}</button> : <button className="button primary" disabled={busy || checking || updating} onClick={() => void install()}>{updating ? <LoaderCircle size={14} className="spin" /> : <Download size={14} />}{t(updating ? '正在更新' : error ? '重试更新' : result.portable ? '更新并安装' : '立即更新')}</button>}{!expanded && !updating && <button className="icon-button" aria-label={t("暂时关闭更新提示")} onClick={() => {setDismissed(result.latestVersion);setProgress(null);}}><X size={16} /></button>}</div>
    </div>}
    {progress && <div className="update-progress" role="status" aria-live="polite"><p>{t(progressText)}</p>{progress.phase==='downloading' && <progress max={100} value={percent ?? undefined} aria-label={t("更新下载进度")} />}</div>}
    {t(error && <p className="update-open-error" role="alert">{t(error)}</p>)}
    {expanded && <div className="update-controls"><p>{t(result?.manualDownload ? '当前安装方式请从官网下载并安装新版。副本和设置会保留。' : "点击“立即更新”后，自动下载、校验、安装并重启应用。配置和修复记录保留。")}</p>{result?.manualDownload ? <button className="button secondary compact" disabled={busy || updating} onClick={() => void downloadManually()}><Download size={14} />{t('打开官网下载页')}</button> : <button className="button secondary compact" disabled={checking || busy || updating} onClick={() => void check()}><RefreshCw size={14} className={checking ? 'spin' : ''} />{t(checking ? '正在检查' : '检查更新')}</button>}</div>}
    <dialog ref={confirmDialog} className="app-dialog" onCancel={()=>setUpdateConfirmation(false)}><h2>{t("关闭副本并更新？")}</h2><p className="modal-subtitle">{t("更新需要重启应用，本地代理也会暂停。请保存浏览器中未完成的输入。")}</p><p>{t("确认后关闭所有运行中的专用副本，再下载、校验并安装更新。副本数据和配置保留。")}</p><div className="modal-actions"><button className="button secondary" onClick={()=>setUpdateConfirmation(false)}>{t("暂不更新")}</button><button className="button primary" onClick={()=>void install(true)}>{t("关闭副本并更新")}</button></div></dialog>
  </section>;
}
