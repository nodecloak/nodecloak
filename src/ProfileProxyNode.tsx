import {useState} from 'react';
import {ArrowDownToLine,Network} from 'lucide-react';
import {parseProxyUri,type ProfileDraft} from './browser-profiles';
import {t} from './i18n';
import './proxy-chain.css';

export default function ProfileProxyNode({draft,hasSecret,onChange,disabled=false}:{draft:ProfileDraft;hasSecret?:boolean;onChange:(draft:ProfileDraft)=>void;disabled?:boolean}) {
 const [link,setLink]=useState(''),[error,setError]=useState('');
 const node=draft.proxy.node;
 const hysteria=draft.proxy.mode==='hysteria2',authLabel=hysteria?'节点认证信息':'节点 UUID';
 return <section className="proxy-node" aria-label={t('节点代理')}>
  <label className="proxy-node-label" htmlFor="profile-node-link">{t('节点链接')}</label>
  <div className="proxy-paste"><input id="profile-node-link" aria-label={t('节点链接')} type="password" autoComplete="new-password" spellCheck={false} maxLength={16384} disabled={disabled} value={link} placeholder="vless://… / vmess://… / hysteria2://… / hy2://…" onChange={e=>{setLink(e.target.value);setError('');}}/><button type="button" className="button secondary compact" disabled={disabled||!link.trim()} onClick={()=>{
   try{const parsed=parseProxyUri(link);if(!parsed.proxy.node)throw new Error('请粘贴 VLESS、VMess 或 Hysteria2 节点链接');onChange({...draft,...parsed,proxy:{...parsed.proxy,preProxy:draft.proxy.preProxy},clearPassword:false});setLink('');setError('');}catch(e){setError(t(e instanceof Error?e.message:'节点链接无效'));}
  }}><ArrowDownToLine size={15}/>{t('导入节点')}</button></div>
  {error&&<p className="profile-field-hint" role="alert">{error}</p>}
  {node?<div className="advanced-card proxy-node-summary" role="status"><strong><Network size={16}/>{hysteria?'Hysteria2':draft.proxy.mode.toUpperCase()} · {draft.proxy.host}:{draft.proxy.port}</strong><dl><div><dt>{t('传输')}</dt><dd>{hysteria?'QUIC / UDP':node.transport.toUpperCase()}</dd></div><div><dt>TLS</dt><dd>{node.insecure?t('跳过证书验证'):node.tls.toUpperCase()}</dd></div>{node.serverName&&<div><dt>SNI</dt><dd>{node.serverName}</dd></div>}</dl></div>:<p className="profile-field-hint">{t('导入节点后可查看连接参数；不支持的传输选项会明确提示。')}</p>}
  <label className="proxy-node-label">{t(authLabel)}<input aria-label={t(authLabel)} type="password" autoComplete="new-password" disabled={disabled} value={draft.password||''} placeholder={t(hasSecret?(hysteria?'已安全保存；留空保留原认证信息':'已安全保存；留空保留原 UUID'):'导入节点时自动填写')} onChange={e=>onChange({...draft,password:e.target.value,clearPassword:false})}/></label>
  <p className="profile-field-hint">{t('仅用于此浏览器副本，不修改系统代理或系统 DNS。节点认证信息保存在系统凭据存储。')}</p>
  {hysteria&&<p className="profile-field-hint">{t('Hysteria2 使用 UDP；链式连接需要前置 SOCKS5 支持 UDP 转发。')}</p>}
 </section>;
}
