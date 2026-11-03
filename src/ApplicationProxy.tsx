import {useEffect,useRef,useState} from 'react';
import {Activity,AppWindow,Check,ChevronDown,Clipboard,LoaderCircle,Network,Plus,Power,RefreshCw,ShieldCheck,Trash2,Pencil,X} from 'lucide-react';
import {applicationProxy,chooseProxyExecutable,testProxyDestination,type AppProxyView,type AppProxyRule,type AppProxyDestination} from './application-proxy';
import {parseProxyUri,type LocalProxy,type ProxyTest} from './browser-profiles';
import {t} from './i18n';
import './application-proxy.css';

const active=(view:AppProxyView|null)=>!!view&&(['starting','running','stopping'].includes(view.runtime.phase)||view.runtime.needsCleanup);
export function ApplicationProxyIndicator(){
 const [status,setStatus]=useState<AppProxyView['runtime']|null>(null);
 useEffect(()=>{let alive=true;const refresh=()=>{void applicationProxy('read').then(v=>{if(alive)setStatus(v.runtime);}).catch(()=>{});};refresh();const timer=window.setInterval(refresh,5000);window.addEventListener('nodecloak-application-proxy-change',refresh);return()=>{alive=false;window.clearInterval(timer);window.removeEventListener('nodecloak-application-proxy-change',refresh);};},[]);
 const label=status?.phase==='running'?'已开启':status?.phase==='starting'?'正在启动':status?.phase==='stopping'||status?.needsCleanup?'正在停止':status?.phase==='error'?'需检查':null;
 return label?<span className="ap-nav-status" title={t(label)}>{t(label)}</span>:null;
}
export default function ApplicationProxy(){
 const [view,setView]=useState<AppProxyView|null>(null),[error,setError]=useState(''),[pending,setPending]=useState('');
 const [editor,setEditor]=useState(false),[editing,setEditing]=useState<AppProxyDestination|null>(null),[name,setName]=useState(''),[uri,setUri]=useState(''),[first,setFirst]=useState(''),[firstMode,setFirstMode]=useState<LocalProxy['mode']>('http');
 const [deleting,setDeleting]=useState<string|null>(null),[tests,setTests]=useState<Record<string,ProxyTest>>({}),[dismissed,setDismissed]=useState<string|null>(null);
 const mounted=useRef(false),epoch=useRef(0),viewRef=useRef(view);viewRef.current=view;
 useEffect(()=>{
  mounted.current=true;
  const initial=epoch.current;
  void applicationProxy('read').then(v=>{if(mounted.current&&epoch.current===initial)setView(v);}).catch(e=>{if(mounted.current)setError(String(e));});
  void applicationProxy('discover').then(v=>{if(mounted.current&&epoch.current===initial)setView(v);}).catch(e=>{if(mounted.current)setError(String(e));});
  const timer=window.setInterval(()=>{if(!active(viewRef.current))return;const e=epoch.current;void applicationProxy('read').then(v=>{if(mounted.current&&e===epoch.current)setView(v);}).catch(()=>{});},2500);
  return()=>{mounted.current=false;window.clearInterval(timer);};
 },[]);
 async function action(kind:string,payload?:unknown){
  if(pending&&!(pending==='start'&&kind==='stop'))return;const e=++epoch.current;setPending(kind);setError('');setDismissed(null);
  try{const next=await applicationProxy(kind,payload);if(mounted.current&&e===epoch.current)setView(next);return next;}
  catch(reason){
   if(mounted.current&&e===epoch.current){setError(String(reason));try{const current=await applicationProxy('read');if(mounted.current&&e===epoch.current)setView(current);}catch{/* Preserve the original operation error. */}}
  }
  finally{if(mounted.current&&e===epoch.current)setPending('');}
 }
 const locked=!!pending||active(view);
 function openEditor(node:AppProxyDestination|null){setEditing(node);setName(node?.name??'');setUri('');setFirst(node?.proxy.preProxy?`${node.proxy.preProxy.host}:${node.proxy.preProxy.port}`:'');setFirstMode(node?.proxy.preProxy?.mode??'http');setEditor(true);setError('');}
 async function saveNode(event:React.FormEvent){
  event.preventDefault();if(locked)return;
  try{
   if(!name.trim())throw new Error('请填写代理节点名称');
   const parsed=uri.trim()?parseProxyUri(uri):editing?{proxy:editing.proxy,password:undefined}:null;
   if(!parsed)throw new Error('请粘贴代理地址或节点链接');
   let preProxy:LocalProxy|null=null;
   if(first.trim()){const pre=parseProxyUri(first,firstMode);if(pre.password||pre.proxy.username)throw new Error('前置代理请填写不带认证的 HTTP 或 SOCKS5 地址');if(!['http','https','socks5'].includes(pre.proxy.mode))throw new Error('前置代理仅支持 HTTP、HTTPS 或 SOCKS5');preProxy={mode:pre.proxy.mode as LocalProxy['mode'],host:pre.proxy.host,port:pre.proxy.port};}
   const saved=await action('saveNode',{id:editing?.id??null,name:name.trim(),proxy:{...parsed.proxy,credentialRef:null,preProxy},password:parsed.password,clearPassword:!!uri.trim()&&!parsed.password});
   if(saved){setUri('');setName('');setFirst('');setEditing(null);setEditor(false);}
  }catch(e){setError(String(e));}
 }
 async function assign(rule:AppProxyRule,proxyId:string){if(!view||locked)return;await action('saveRules',{rules:view.settings.rules.map(r=>r.id===rule.id?{...r,proxyId:proxyId||null,enabled:!!proxyId}:r)});}
 async function choose(rule?:AppProxyRule){
  if(locked||!view)return;setPending('pickExecutable');setError('');
  try{const executable=await chooseProxyExecutable();if(!executable)return;const rules=rule?view.settings.rules.map(r=>r.id===rule.id?{...r,executable}:r):[...view.settings.rules,{id:crypto.randomUUID(),app:'custom' as const,executable,proxyId:null,enabled:false}];setPending('');await action('saveRules',{rules});}
  catch(e){setError(String(e));}finally{setPending('');}
 }
 async function testNode(id:string){if(pending)return;setPending(`test:${id}`);setError('');try{const result=await testProxyDestination(id);setTests(v=>({...v,[id]:result}));}catch(e){setError(String(e));}finally{setPending('');}}
 const running=active(view),supported=view?.supported??true;
 const phase=view?.runtime.phase;
 const status=phase==='running'?'已开启':phase==='starting'?'正在启动':phase==='stopping'?'正在停止':view?.runtime.needsCleanup?'等待释放网络接口':'未开启';
 return <section className="application-proxy" aria-label={t('应用代理')}>
  <div className="ap-hero"><div className="ap-hero-icon"><Network size={25}/></div><div className="ap-hero-copy"><span className="ap-eyebrow">YOUR APPS. YOUR ROUTES.</span><h2>{t('每个应用，独立连接。')}</h2><p>{t('为应用指定代理节点，其他应用保持直连。浏览器副本继续使用原来的代理配置。')}</p></div><div className="ap-control"><span className={`ap-state ${phase==='running'?'is-running':''}`}><i/>{t(status)}</span><button className={`button ${running?'secondary':'primary'}`} disabled={!!pending||!supported||!view} onClick={()=>void action(running?'stop':'start')}>
   {pending==='start'||pending==='stop'?<LoaderCircle size={16} className="spin"/>:<Power size={16}/>}{t(pending==='start'?'正在请求授权并启动':pending==='stop'?'正在停止':running?'停止应用代理':'开启应用代理')}
  </button>{pending==='start'&&<button className="ap-link" onClick={()=>void action('stop')}>{t('取消启动')}</button>}</div></div>
  {!supported&&<div className="ap-notice" role="status">{t('应用代理当前仅支持 Windows。浏览器副本的代理功能仍可正常使用。')}</div>}
  {view?.simulation&&<div className="ap-notice">{t('交互演示，不会修改电脑网络。')}</div>}
  <div className="ap-consent"><ShieldCheck size={17}/><p>{t('开启时将请求 Windows 管理员授权，并临时创建 TUN 网络接口。停止后释放该接口及对应路由；系统 DNS 和 HTTP 代理设置保持不变。')}{' '}{t('切换路由可能短暂重连。应用自身的代理设置和本机回环连接不会被覆盖。')}</p></div>
  {(error||view?.runtime.error)&&(error||view?.runtime.error)!==dismissed&&<div className="ap-error" role="alert">{t(error||view?.runtime.error)}<button type="button" aria-label={t('关闭提示')} onClick={()=>{setDismissed(error||view?.runtime.error||null);setError('');}}><X size={15}/></button></div>}
  {!view&&!error&&<p role="status" className="ap-loading"><LoaderCircle size={15} className="spin"/>{t('正在读取应用代理设置')}</p>}
  <div className="ap-grid"><section className="ap-card"><header><div><h3><AppWindow size={17}/>{t('应用分流')}</h3><p>{t('选择节点后保存规则，开启应用代理时生效。')}</p></div><div className="ap-actions"><button className="button secondary compact" disabled={!!pending||!supported} onClick={()=>void action('discover')}><RefreshCw size={14} className={pending==='discover'?'spin':''}/>{t('重新识别')}</button><button className="button secondary compact" disabled={locked||!supported} onClick={()=>void choose()}><Plus size={14}/>{t('添加应用')}</button></div></header>
   <div className="ap-rule-list">{view?.settings.rules.map(rule=>{const app=view.apps.find(a=>a.id===rule.app);const title=rule.app==='custom'?rule.executable.split(/[\\/]/).pop()??t('自定义应用'):app?.name??rule.app;const paths=rule.executable?[rule.executable]:app?.paths??[];return <div className="ap-rule" key={rule.id}><span className="ap-app-icon"><AppWindow size={20}/></span><div className="ap-rule-app"><strong>{title}</strong><small>{paths.length?(app?.running?`${app.running} ${t('个进程正在运行')}`:t('已找到程序')):view.discoveryChecked?t('未找到程序'):t('正在识别应用')}</small>{paths.length>0&&<details><summary>{t('查看程序路径')}<ChevronDown size={12}/></summary>{paths.map(path=><code key={path}>{path}</code>)}</details>}</div><div className="ap-assignment"><label className="sr-only" htmlFor={`ap-node-${rule.id}`}>{t('为应用选择代理')} {title}</label><select id={`ap-node-${rule.id}`} value={rule.enabled?rule.proxyId??'':''} disabled={locked||!supported} onChange={e=>void assign(rule,e.target.value)}><option value="">{t('直连')}</option>{view.settings.proxies.map(node=><option key={node.id} value={node.id}>{node.name}</option>)}</select>{!paths.length&&<button className="ap-link" disabled={locked||!supported} onClick={()=>void choose(rule)}>{t('选择程序')}</button>}{rule.app==='custom'&&<button className="ap-link danger" disabled={locked} onClick={()=>void action('saveRules',{rules:view.settings.rules.filter(r=>r.id!==rule.id)})}>{t('移除规则')}</button>}</div></div>;})}</div>
   {running&&<p className="ap-footnote">{t('修改规则前请先停止应用代理。')}</p>}
  </section><section className="ap-card ap-nodes"><header><div><h3><Network size={17}/>{t('代理节点')}</h3><p>HTTP / SOCKS5 / VLESS / VMess / HY2</p></div><button className="ap-icon-button" title={t('添加节点')} aria-label={t('添加节点')} disabled={locked||!supported} onClick={()=>openEditor(null)}><Plus size={18}/></button></header>
   {!view?.settings.proxies.length&&!editor&&<div className="ap-empty"><Network size={28}/><strong>{t('先添加一个代理节点')}</strong><p>{t('支持代理地址和节点链接，凭据单独保存在系统凭据存储中。')}</p><button className="button secondary" disabled={locked||!supported} onClick={()=>openEditor(null)}><Plus size={15}/>{t('添加节点')}</button></div>}
   {view?.settings.proxies.map(node=><div className="ap-node" key={node.id}><div><strong>{node.name}</strong><small>{node.proxy.mode.toUpperCase()} · {node.proxy.host}:{node.proxy.port}</small>{tests[node.id]&&<small className="ap-test-result"><Check size={12}/>{t('节点检测')} · {tests[node.id].ip??t('未读取到出口 IP')} · {tests[node.id].latency} ms</small>}</div><div className="ap-node-tools"><button className="ap-link" disabled={!!pending||!supported} onClick={()=>void testNode(node.id)}>{pending===`test:${node.id}`?t('检测中'):t('检测节点')}</button><button className="ap-icon-button" aria-label={`${t('编辑节点')} ${node.name}`} disabled={locked} onClick={()=>openEditor(node)}><Pencil size={14}/></button><button className="ap-icon-button" aria-label={`${t('删除节点')} ${node.name}`} disabled={locked} onClick={()=>setDeleting(node.id)}><Trash2 size={14}/></button></div>{deleting===node.id&&<div className="ap-delete-confirm"><p>{t('删除后，使用此节点的应用将恢复直连规则。')}</p><button className="ap-link danger" onClick={()=>{setDeleting(null);void action('deleteNode',{id:node.id});}}>{t('确认删除')}</button><button className="ap-link" onClick={()=>setDeleting(null)}>{t('取消')}</button></div>}</div>)}
   {editor&&<form className="ap-node-form" onSubmit={event=>void saveNode(event)}><h4>{t(editing?'编辑代理节点':'添加代理节点')}</h4><label>{t('节点名称')}<input value={name} maxLength={60} required disabled={locked} onChange={e=>setName(e.target.value)} autoComplete="off"/></label><label>{t('代理地址或节点链接')}<input type="password" value={uri} disabled={locked} onChange={e=>setUri(e.target.value)} autoComplete="off" spellCheck={false} placeholder={editing?t('留空保留原连接信息'):'socks5://… / vless://… / hy2://…'}/></label><details><summary>{t('前置代理（可选）')}</summary><label>{t('协议')}<select value={firstMode} disabled={locked} onChange={e=>setFirstMode(e.target.value as LocalProxy['mode'])}><option value="http">HTTP</option><option value="https">HTTPS</option><option value="socks5">SOCKS5</option></select></label><label>{t('本机或软路由代理地址')}<input value={first} disabled={locked} onChange={e=>setFirst(e.target.value)} placeholder="192.168.10.1:7890" autoComplete="off"/></label><p>{t('连接顺序：应用 → 前置代理 → 指定节点。HY2 前置代理需要 SOCKS5 UDP 支持。')}</p></details><div className="ap-actions"><button className="button secondary compact" type="button" disabled={!!pending} onClick={()=>{setUri('');setEditor(false);setEditing(null);}}>{t('取消')}</button><button className="button primary compact" disabled={locked} type="submit">{pending==='saveNode'?<LoaderCircle size={14} className="spin"/>:<Check size={14}/>} {t('保存节点')}</button></div></form>}
  </section></div>
  <section className="ap-card ap-evidence"><header><div><h3><Activity size={17}/>{t('连接与诊断')}</h3><p>{t('仅显示命中应用规则的连接，不代表应用登录或服务一定可用。')}</p></div><button className="button secondary compact" disabled={!!pending} onClick={()=>void action('read')}><RefreshCw size={14}/>{t('刷新')}</button></header>
   {!!view?.runtime.connections.length?<div className="ap-table-wrap"><table><thead><tr><th>{t('应用')}</th><th>{t('代理节点')}</th><th>{t('目标')}</th><th>{t('协议')}</th><th>{t('收 / 发')}</th></tr></thead><tbody>{view.runtime.connections.map(c=><tr key={c.id}><td>{c.app}</td><td>{view.settings.proxies.find(p=>p.id===c.proxyId)?.name??c.proxyId}</td><td>{c.destination}</td><td>{c.network.toUpperCase()}</td><td>{Math.round(c.download/1024)} / {Math.round(c.upload/1024)} KB</td></tr>)}</tbody></table></div>:<div className="ap-connection-empty">{t(running?'尚未观察到匹配连接。请在应用中重新连接后刷新。':'应用代理尚未开启，程序仍使用原来的网络连接。')}</div>}
   <details className="ap-diagnostics"><summary>{t('运行记录')}<ChevronDown size={14}/><span>{view?.runtime.events.length??0}</span></summary>{view?.runtime.events.length?<ol>{[...view.runtime.events].reverse().map((e,i)=><li key={`${e.at}-${i}`}><time>{new Date(e.at).toLocaleTimeString()}</time><span>{e.message}</span></li>)}</ol>:<p>{t('暂无运行记录')}</p>}<button className="ap-link" onClick={()=>void navigator.clipboard.writeText(JSON.stringify({runtime:view?.runtime,applications:view?.apps},null,2)).catch(e=>setError(String(e)))}><Clipboard size={13}/>{t('复制诊断')}</button></details>
   {view?.runtime.warnings.map(w=><p className="ap-footnote" key={w}>{w}</p>)}
  </section>
 </section>;
}
