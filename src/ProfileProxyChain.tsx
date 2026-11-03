import {ArrowRight,Route} from 'lucide-react';
import type {LocalProxy,ProxyConfig} from './browser-profiles';
import {t} from './i18n';
import SelectMenu from './SelectMenu';
import Toggle from './Toggle';
import './proxy-chain.css';

const address=(host:string,port:number)=>`${host.includes(':')?`[${host}]`:host||t('待填写')}:${port||'…'}`;
export default function ProfileProxyChain({proxy,onChange,disabled=false}:{proxy:ProxyConfig;onChange:(proxy:ProxyConfig)=>void;disabled?:boolean}) {
 const local=proxy.preProxy;
 const hysteria=proxy.mode==='hysteria2';
 const update=(patch:Partial<LocalProxy>)=>{if(local)onChange({...proxy,preProxy:{...local,...patch}});};
 return <section className="advanced-card proxy-chain-card" aria-label={t('链式代理')}>
  <label className="advanced-toggle"><span><strong><Route size={16} aria-hidden="true"/>{t('链式代理')}</strong><small>{t('通过本机或局域网代理连接副本代理。')}</small></span><Toggle aria-label={t('启用链式代理')} disabled={disabled} checked={Boolean(local)} onChange={e=>onChange({...proxy,preProxy:e.target.checked?{mode:hysteria?'socks5':'http',host:'127.0.0.1',port:7890}:undefined})}/></label>
  {local&&<>
   <div className="profile-form proxy-chain-fields">
    <label>{t('前置代理协议')}<SelectMenu label={t('前置代理协议')} disabled={disabled} value={local.mode} options={(hysteria?['socks5']:['http','https','socks5']).map(mode=>({value:mode,label:mode.toUpperCase()}))} onChange={mode=>update({mode:mode as LocalProxy['mode']})}/></label>
    <label>{t('前置代理地址')}<input aria-label={t('前置代理地址')} autoComplete="off" spellCheck={false} disabled={disabled} value={local.host} placeholder="127.0.0.1" onChange={e=>update({host:e.target.value.trim()})}/></label>
    <label>{t('前置代理端口')}<input aria-label={t('前置代理端口')} type="number" min={1} max={65535} disabled={disabled} value={local.port||''} placeholder="7890" onChange={e=>update({port:Number(e.target.value)})}/></label>
   </div>
   <p className="profile-field-hint">{t('填写本机代理或软路由的监听地址与端口。透明代理网关无需额外配置前置代理。')}</p>
   {hysteria&&local.mode!=='socks5'&&<p className="profile-error" role="alert">{t('Hysteria2 需要支持 UDP 的 SOCKS5 前置代理')}</p>}
   <ol className="proxy-chain-route" aria-label={t('代理链路')}>
    <li><span>{t('浏览器副本')}</span></li><li className="proxy-chain-arrow" aria-hidden="true"><ArrowRight size={14}/></li>
    <li><span>{t('前置代理')}</span><code>{local.mode.toUpperCase()} · {address(local.host,local.port)}</code></li><li className="proxy-chain-arrow" aria-hidden="true"><ArrowRight size={14}/></li>
    <li><span>{t('副本代理')}</span><code>{proxy.mode.toUpperCase()} · {proxy.apiUrl!=null?t('API 提取'):address(proxy.host,proxy.port)}</code></li><li className="proxy-chain-arrow" aria-hidden="true"><ArrowRight size={14}/></li>
    <li><span>{t('目标网站')}</span></li>
   </ol>
  </>}
 </section>;
}
