import {t} from './i18n';
import {useEffect,useRef,useState} from 'react';
import {ArrowUpRight,Check,Clock3,Eye,EyeOff,FolderOpen,Info,LoaderCircle,Plus,RefreshCw,Save,Terminal,Trash2,Wifi} from 'lucide-react';
import {native} from './bridge';
import SelectMenu from './SelectMenu';
import {terminalLocaleChoices,ianaTimezoneChoices} from './locale-options';
const timezoneOptions=ianaTimezoneChoices();
import {getTerminalCatalog,getTerminalEnvironment,launchTerminal,openTerminalGuide,saveTerminalEnvironment,selectTerminalDirectory,terminalEnvironmentDefaults,terminalTools,validateTerminalEnvironment,type TerminalCatalog,type TerminalEnvironment,type TerminalTool} from './terminal';

export default function TerminalWorkspace({busy,onBusyChange,onNotify}:{busy:boolean;onBusyChange:(message:string|null)=>void;onNotify:(message:string)=>void}) {
  const [selected,setSelected]=useState<TerminalTool>('claude');
  const [catalog,setCatalog]=useState<TerminalCatalog|null>(null);
  const [directory,setDirectory]=useState(()=>localStorage.getItem('nodecloak-terminal-directory')||'');
  const [environments,setEnvironments]=useState<Record<TerminalTool,TerminalEnvironment>>(()=>Object.fromEntries(terminalTools.map(item=>[item.id,terminalEnvironmentDefaults()])) as Record<TerminalTool,TerminalEnvironment>);
  const loaded=useRef(new Set<TerminalTool>());
  const [loadingEnvironment,setLoadingEnvironment]=useState(true);
  const [showValues,setShowValues]=useState(false);
  const [loading,setLoading]=useState(true);
  const [error,setError]=useState('');
  const [selectingDirectory,setSelectingDirectory]=useState(false);
  useEffect(()=>{let cancelled=false;getTerminalCatalog().then(result=>{if(!cancelled)setCatalog(result);}).catch(error=>{if(!cancelled)setError(String(error));}).finally(()=>{if(!cancelled)setLoading(false);});return()=>{cancelled=true;};},[]);
  useEffect(()=>{
    setShowValues(false);
    if(loaded.current.has(selected)){setLoadingEnvironment(false);return;}
    let cancelled=false;setLoadingEnvironment(true);
    getTerminalEnvironment(selected).then(environment=>{if(!cancelled){setEnvironments(all=>({...all,[selected]:environment}));loaded.current.add(selected);}}).catch(error=>{if(!cancelled)setError(String(error));}).finally(()=>{if(!cancelled)setLoadingEnvironment(false);});
    return()=>{cancelled=true;};
  },[selected]);
  const tool=terminalTools.find(item=>item.id===selected)!;
  const environment=environments[selected];
  const installed=catalog?.tools.find(item=>item.id===selected)?.installed;
  const customPath=environment.mode==='custom'&&environment.variables.some(v=>v.name.toUpperCase()==='PATH'&&v.value);
  const disabled=busy||loadingEnvironment;
  const update=(patch:Partial<TerminalEnvironment>)=>setEnvironments(all=>({...all,[selected]:{...all[selected],...patch}}));
  const variable=(index:number,patch:Partial<TerminalEnvironment['variables'][number]>)=>update({variables:environment.variables.map((row,i)=>i===index?{...row,...patch}:row)});
  async function refresh(){setLoading(true);setError('');try{setCatalog(await getTerminalCatalog());}catch(error){setError(String(error));}finally{setLoading(false);}}
  async function launch(){
    const issue=validateTerminalEnvironment(environment);if(issue){setError(issue);return;}
    setError('');onBusyChange(`正在打开 ${tool.name}`);
    try{await launchTerminal(selected,directory,environment);localStorage.setItem('nodecloak-terminal-directory',directory);onNotify(`已打开 ${tool.name} 终端。`);}catch(error){setError(String(error));}finally{onBusyChange(null);}
  }
  async function save(){
    const issue=validateTerminalEnvironment(environment);if(issue){setError(issue);return;}
    setError('');onBusyChange(t('正在保存终端环境'));
    try{await saveTerminalEnvironment(selected,environment);loaded.current.add(selected);onNotify(t('终端环境已保存。'));}catch(error){setError(String(error));}finally{onBusyChange(null);}
  }
  async function guide(){try{await openTerminalGuide(selected);}catch(error){setError(String(error));}}
  async function chooseDirectory(){setSelectingDirectory(true);setError('');try{const path=await selectTerminalDirectory(directory,t('选择工作目录'));if(path!==null)setDirectory(path);}catch(error){setError(String(error));}finally{setSelectingDirectory(false);}}
  const preview=environment.mode==='custom'?[...Object.entries({TZ:environment.timezone,LANG:environment.lang,LC_ALL:environment.lcAll}).filter(([,value])=>value),...environment.variables.filter(v=>v.name).map(v=>[v.name,'••••••••'])]:[];
  return <div className="terminal-workspace">
    <section className="terminal-tools-section"><div className="section-heading"><h3>{t('终端工具')}</h3><button className="button secondary small-button" disabled={busy||loading} onClick={()=>void refresh()}><RefreshCw size={14} className={loading?'spin':''}/>{t('重新检测')}</button></div>
      <div className="terminal-tools" aria-label={t('选择终端工具')}>{terminalTools.map(item=>{
        const available=catalog?.tools.find(t=>t.id===item.id)?.installed;
        return <button key={item.id} type="button" className={`terminal-tool ${selected===item.id?'selected':''}`} aria-pressed={selected===item.id} disabled={busy} onClick={()=>{setSelected(item.id);setError('');}}><span className="terminal-tool-brand">{t(item.brand)}</span><strong>{t(item.name)}</strong><code>{t(item.command||'PowerShell / zsh')}</code><span className={`terminal-tool-status ${available?'available':''}`}>{loading?<LoaderCircle size={12} className="spin"/>:available?<Check size={12}/>:<Info size={12}/>} {t(loading?'正在检测':!native?available?'可启动（演示）':'未安装（演示）':available?item.id==='shell'?'系统自带':'已安装':'未安装')}</span></button>;
      })}</div>
    </section>
    <section className="terminal-launch-panel settings-card"><div className="terminal-launch-heading"><h3>{t(tool.name)}</h3><span className="tiny-label">LOCAL</span></div>
      <div className="terminal-directory"><label htmlFor="terminal-working-directory"><FolderOpen size={15}/>{t('工作目录')}</label><div className="terminal-directory-controls"><input id="terminal-working-directory" aria-label={t('终端工作目录')} value={directory} onChange={e=>setDirectory(e.target.value)} placeholder={t(catalog?.homeDirectory||'留空使用用户主目录')} disabled={busy||selectingDirectory}/><button type="button" className="button secondary" disabled={busy||selectingDirectory||!native} title={!native?t('请在桌面版中选择工作目录。'):undefined} onClick={()=>void chooseDirectory()}>{selectingDirectory?<LoaderCircle size={15} className="spin"/>:<FolderOpen size={15}/>} {t('选择文件夹')}</button></div><small>{t('填写项目文件夹的完整路径；留空使用用户主目录。')}{t(catalog?.homeDirectory&&` 默认：${catalog.homeDirectory}`)}</small></div>
      <div className="terminal-environment-heading"><div><h4>{t('启动环境')}</h4><p>{t('为每个终端工具分别设置，仅作用于新启动的终端。')}</p></div><SelectMenu label={t('环境模式')} value={environment.mode} onChange={mode=>update({mode:mode as TerminalEnvironment['mode']})} disabled={disabled} options={[{value:'inherit',label:t('继承系统环境')},{value:'custom',label:t('自定义环境')}]}/></div>
      {environment.mode==='custom'&&<fieldset className="terminal-environment-fields" disabled={disabled}>
        <div className="terminal-locale-fields">{([['timezone','时区'],['lang','语言（LANG）'],['lcAll','区域格式（LC_ALL）']] as const).map(([key,label])=><label key={key}>{t(label)}<SelectMenu label={t(label)} value={environment[key]} disabled={disabled} searchable allowCustom searchMaxLength={100} placeholder={t('搜索或输入自定义值')} onChange={value=>update({[key]:value})} options={[{value:'',label:t('继承已有值')},...(key==='timezone'?timezoneOptions:terminalLocaleChoices)]}/></label>)}</div><p className="terminal-field-hint">{t('可搜索选择或输入自定义值；选择“继承已有值”沿用当前环境。')}</p>
        <div className="terminal-variables-heading"><h4>{t('环境变量')}</h4><button type="button" className="button secondary compact" disabled={disabled} onClick={()=>setShowValues(value=>!value)}>{showValues?<EyeOff size={14}/>:<Eye size={14}/>} {t(showValues?'隐藏变量值':'显示变量值')}</button></div>
        <div className="terminal-variable-list">{environment.variables.map((row,index)=><div className="terminal-variable-row" key={index}><input aria-label={`${t('变量名')} ${index+1}`} placeholder="HTTPS_PROXY" value={row.name} autoComplete="off" spellCheck={false} onChange={e=>variable(index,{name:e.target.value})}/><input aria-label={`${t('变量值')} ${index+1}`} type={showValues?'text':'password'} placeholder={t('变量值')} value={row.value} autoComplete="off" onChange={e=>variable(index,{value:e.target.value})}/><button type="button" className="icon-button danger" aria-label={`${t('删除变量')} ${index+1}`} onClick={()=>update({variables:environment.variables.filter((_,i)=>i!==index)})}><Trash2 size={16}/></button></div>)}</div>
        <button type="button" className="button secondary compact" disabled={disabled||environment.variables.length>=32} onClick={()=>update({variables:[...environment.variables,{name:'',value:''}]})}><Plus size={14}/>{t('添加环境变量')}</button><p className="terminal-field-hint">{t('可设置代理、API 地址和密钥。配置在本机加密保存，预览始终隐藏变量值。')}</p>
      </fieldset>}
      {environment.mode==='inherit'&&<p className="terminal-inherit-note">{t('沿用已有时区、语言和环境变量。')}</p>}
      {error&&<div className="notice warning-notice" role="alert"><Info size={16}/><p>{t(error)}</p></div>}
      {!loading&&catalog&&!installed&&!customPath&&<div className="notice"><Info size={16}/><p>{t('未找到')}{t(tool.name)}{t('。请先按官方指南安装，再点击“重新检测”。首次运行时，请在终端中完成登录。')}</p></div>}
      <div className="workspace-actions"><button className="button primary" disabled={disabled||loading||(!installed&&!customPath)} onClick={()=>void launch()}>{busy?<LoaderCircle size={15} className="spin"/>:<Terminal size={15}/>}{t('打开')}{t(tool.name)}<ArrowUpRight size={15}/></button><button className="button secondary" disabled={disabled} onClick={()=>void save()}><Save size={15}/>{t('保存环境')}</button>{tool.guide&&<button className="button secondary" disabled={busy} onClick={()=>void guide()}><ArrowUpRight size={14}/>{t('官方安装指南')}</button>}</div>
    </section>
    <div className="terminal-card" aria-label={t('启动环境预览')}><div className="terminal-top"><span className="terminal-dots"><i/><i/><i/></span><span>{t('NodeCloak 启动环境')}</span><span>{t('只影响当前进程')}</span></div><div className="terminal-body"><p><span className="terminal-comment">{t('# 从所选工作目录启动')}</span></p>{!preview.length&&<p><span className="terminal-comment">{t('# 继承已有环境')}</span></p>}{preview.map(([name,value],i)=><p key={`${name}-${i}`}><span>{name}</span><span className="terminal-equals">=</span>{value}</p>)}<p className="terminal-command"><span>❯</span> {t(tool.command||'你的 CLI 命令')}<span className="terminal-caret"/></p></div></div>
    <div className="info-grid"><article className="info-card"><Clock3 size={20}/><h3>{t('独立的进程环境')}</h3><p>{t('自定义时区、语言和环境变量，仅作用于这个终端及其启动的进程。')}</p></article><article className="info-card"><Wifi size={20}/><h3>{t('沿用已有连接')}</h3><p>{t('继承已有代理与 CLI 配置，自定义变量可覆盖对应值。')}</p></article></div>
    <div className="notice"><Info size={17}/><p>{t('Windows 使用 PowerShell，macOS 使用 Terminal，Ubuntu 使用已安装的桌面终端和 Bash。工具的安装、登录和权限由各 CLI 自身管理。')}{t(!native&&'当前为界面演示，安装状态是示例数据。')}</p></div>
  </div>;
}
