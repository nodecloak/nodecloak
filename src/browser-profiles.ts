import {invoke} from '@tauri-apps/api/core';
import {native} from './bridge';
import type {BrowserId,Check} from './domain';
import type {SharedProfileGrant} from './profile-sharing';
import {fingerprintDefaults,fingerprintNeedsInjection,type FingerprintSettings} from './profile-fingerprint';
import {isNodeMode,parseNodeUri,validNode,type NodeOptions} from './proxy-node';
export type ProxyMode='direct'|'system'|'http'|'https'|'socks5'|'vless'|'vmess';
export interface LocalProxy {mode:'http'|'https'|'socks5';host:string;port:number}
export interface ProxyConfig {mode:ProxyMode;host:string;port:number;username:string;credentialRef?:string|null;preProxy?:LocalProxy|null;apiUrl?:string|null;node?:NodeOptions|null}
export type PermissionMode='ask'|'block';
export interface RegionalSettings {enabled:boolean;languageMode:'ip'|'custom';timezoneMode:'system'|'ip'|'custom';timezone:string;locationMode:'system'|'ip'|'custom';latitude:number;longitude:number;accuracy:number}
export interface IpRegion {ip:string;country:string;city:string;language:string;timezone:string;latitude:number;longitude:number;checkedAt:string}
export interface ResolvedRegion {language:string;timezone:string|null;latitude:number|null;longitude:number|null;accuracy:number;ipRegion:IpRegion|null}
export const regionalDefaults=():RegionalSettings=>({enabled:false,languageMode:'custom',timezoneMode:'system',timezone:'UTC',locationMode:'system',latitude:1.3521,longitude:103.8198,accuracy:100});
export const needsRegionalControl=(p:Preferences)=>p.regional.languageMode==='ip'||p.regional.timezoneMode!=='system'||p.regional.locationMode!=='system'||p.advanced.hardwareConcurrency>0||p.advanced.viewportWidth>0;
export const requiresBrowserControl=(browser:BrowserId,p:Preferences)=>browser!=='firefox'&&(fingerprintNeedsInjection(p.fingerprint)||needsRegionalControl(p));
export interface AdvancedSettings {notifications:PermissionMode;location:PermissionMode|'allow';camera:PermissionMode;microphone:PermissionMode;blockImages:boolean;blockWebgl:boolean;resistFingerprinting:boolean;doNotTrack:'inherit'|'on'|'off';webRtc:'inherit'|'default'|'public'|'proxy';blockWebgpu:boolean;blockSpeech:boolean;blockBattery:boolean;canvasProtection:boolean;hardwareConcurrency:number;viewportWidth:number;viewportHeight:number}
export const advancedDefaults=():AdvancedSettings=>({notifications:'ask',location:'ask',camera:'ask',microphone:'ask',blockImages:false,blockWebgl:false,resistFingerprinting:false,doNotTrack:'inherit',webRtc:'inherit',blockWebgpu:false,blockSpeech:false,blockBattery:false,canvasProtection:false,hardwareConcurrency:0,viewportWidth:0,viewportHeight:0});
export interface Preferences {language:string;privacy:boolean;fontRestriction:boolean;startupUrl:string;advanced:AdvancedSettings;regional:RegionalSettings;fingerprint?:FingerprintSettings}
export function normalizePreferences(value:Partial<Preferences>):Preferences {
  const advanced={...advancedDefaults(),...value.advanced};
  // Old "allow" permissions applied to a simulated position, not the actual device.
  if(!value.regional?.enabled&&value.regional?.locationMode&&value.regional.locationMode!=='system'&&advanced.location==='allow')advanced.location='ask';
  return {...defaults().preferences,...value,fingerprint:value.fingerprint,advanced,regional:value.regional?.enabled?{...regionalDefaults(),...value.regional}:{...regionalDefaults(),languageMode:value.regional?.languageMode??'custom'}};
}
export interface ProxyTest {ip:string|null;country:string|null;latency:number;checkedAt:string;connection:string;target:string;clock?:Check|null}
export interface BrowserProfile {id:string;name:string;browser:BrowserId;notes:string;tags:string[];proxy:ProxyConfig;preferences:Preferences;createdAt:string;updatedAt:string;deletedAt:string|null;lastTest:ProxyTest|null;legacy:boolean;running:boolean;pendingRestart:boolean;browserAvailable:boolean;hasPassword:boolean;proxyReady:boolean;activeProxyEndpoint?:{host:string;port:number}|null;lastRegion?:ResolvedRegion|null;regionalReady?:boolean;regionalError?:string|null;sharing?:SharedProfileGrant|null}
export interface ProfileDraft {name:string;browser:BrowserId;notes:string;tags:string[];proxy:ProxyConfig;password?:string;clearPassword:boolean;preferences:Preferences}
export const defaults=():ProfileDraft=>({name:'',browser:'firefox',notes:'',tags:[],proxy:{mode:'system',host:'',port:8080,username:''},password:'',clearPassword:false,preferences:{language:'en-US,en',privacy:true,fontRestriction:false,startupUrl:'https://ip.nodecloak.com/',advanced:advancedDefaults(),regional:regionalDefaults(),fingerprint:fingerprintDefaults()}});
export function parseProxyUri(input:string,fallbackMode:ProxyMode='http'):{proxy:ProxyConfig;password:string} {
  if(/^(vless|vmess):\/\//.test(input.trim()))return parseNodeUri(input);
  const value=input.trim(),invalid=()=>new Error('代理链接协议或格式无效');
  if(!value||value.length>8192||/[\x00-\x1f\x7f]/.test(value))throw invalid();
  let mode:ProxyMode,host:string,port:number,username:string,password:string;
  if(/^[a-z][a-z0-9+.-]*:\/\//i.test(value)){
    let url:URL;try{url=new URL(value);}catch{throw invalid();}
    const protocol=url.protocol.slice(0,-1);
    if(!['http','https','socks5'].includes(protocol)||!url.hostname||url.pathname!=='/'&&url.pathname!==''||url.search||url.hash)throw invalid();
    mode=protocol as ProxyMode;host=url.hostname.replace(/^\[|\]$/g,'');
    port=Number(url.port||({http:80,https:443,socks5:1080}[protocol]));
    try{username=decodeURIComponent(url.username);password=decodeURIComponent(url.password);}catch{throw invalid();}
  }else{
    // Split host/port/user once, retaining the complete raw password (including
    // colons, @ and percent signs). Brackets disambiguate IPv6 from field separators.
    const parts=/^(?:\[([^\]]+)\]|([^:[\]\s]+)):(\d+)(?::([^:]*):(.*))?$/s.exec(value);
    if(!parts)throw new Error('请粘贴代理链接或 主机:端口:用户名:密码');
    host=parts[1]??parts[2];port=Number(parts[3]);username=parts[4]??'';password=parts[5]??'';
    if(parts[1]&&!host.includes(':'))throw invalid();
    mode=['http','https','socks5'].includes(fallbackMode)?fallbackMode:'http';
  }
  if(!Number.isInteger(port)||port<1||port>65535)throw new Error('代理端口必须在 1–65535 之间');
  if(!host||host.length>253||/[\s/\\@?#]/.test(host))throw invalid();
  if(host.includes(':')){try{new URL(`http://[${host}]/`);}catch{throw invalid();}}
  else if(!host.split('.').every(label=>label.length>0&&label.length<=63&&/^[a-z0-9-]+$/i.test(label)))throw invalid();
  if(new TextEncoder().encode(username).length>255||/[:\x00-\x1f\x7f]/.test(username))throw new Error('代理用户名格式无效');
  if(new TextEncoder().encode(password).length>255||/[\x00-\x1f\x7f]/.test(password))throw new Error('代理密码格式无效或超过 255 字节');
  return {proxy:{mode,host,port,username},password};
}
export function validateDraft(draft:ProfileDraft):string|null {
  draft={...draft,preferences:normalizePreferences(draft.preferences)};
  if(!draft.name.trim()||draft.name.length>60)return '请填写 1–60 个字符的副本名称';
  if(draft.tags.length>10||draft.tags.some(tag=>!tag.trim()||tag.length>24))return '最多 10 个标签，每个标签不超过 24 个字符';
  if(draft.notes.length>1000)return '备注最多 1000 个字符';
  const advanced=draft.preferences.advanced;
  const fingerprint=draft.preferences.fingerprint;
  if(fingerprint&&(fingerprint.version!==1||!/^[0-9a-f]{64}$/.test(fingerprint.seed)))return '指纹种子或版本无效';
  if(fingerprint?.enabled&&(advanced.resistFingerprinting||advanced.canvasProtection))return '稳定指纹与 Firefox 原生随机保护不能同时开启';
  if(![0,2,4,8,16].includes(advanced.hardwareConcurrency)||!Number.isInteger(advanced.viewportWidth)||!Number.isInteger(advanced.viewportHeight)||((advanced.viewportWidth!==0||advanced.viewportHeight!==0)&&(advanced.viewportWidth<320||advanced.viewportWidth>3840||advanced.viewportHeight<200||advanced.viewportHeight>2160)))return '硬件提示或页面视口尺寸无效';
  if(!['inherit','on','off'].includes(advanced.doNotTrack)||!['inherit','default','public','proxy'].includes(advanced.webRtc)||[advanced.blockWebgpu,advanced.blockSpeech,advanced.blockBattery,advanced.canvasProtection].some(v=>typeof v!=='boolean'))return '隐私设置格式无效';
  if(!advanced||[advanced.notifications,advanced.camera,advanced.microphone].some(v=>!['ask','block'].includes(v))||!['ask','allow','block'].includes(advanced.location)||[advanced.blockImages,advanced.blockWebgl,advanced.resistFingerprinting].some(v=>typeof v!=='boolean'))return '高级设置格式无效';
  const r=draft.preferences.regional;
  if(!r||!['ip','custom'].includes(r.languageMode)||!['system','ip','custom'].includes(r.timezoneMode)||!['system','ip','custom'].includes(r.locationMode))return '区域匹配方式无效';
  if(r.timezoneMode==='custom'){try{new Intl.DateTimeFormat('en-US',{timeZone:r.timezone});}catch{return '请填写有效的 IANA 时区，例如 America/Los_Angeles';}if(!r.timezone.trim())return '请填写时区';}
  if(!Number.isFinite(r.latitude)||!Number.isFinite(r.longitude)||!Number.isFinite(r.accuracy)||Math.abs(r.latitude)>90||Math.abs(r.longitude)>180||r.accuracy<1||r.accuracy>100000)return '经纬度或定位精度无效';
  if(advanced.resistFingerprinting&&needsRegionalControl(draft.preferences))return '独立区域设置与 Firefox 严格指纹保护冲突，请关闭严格保护';
  if(draft.browser!=='firefox'&&(advanced.canvasProtection||advanced.blockBattery||advanced.resistFingerprinting||draft.preferences.fontRestriction))return '字体限制、Canvas 保护、电池保护和严格指纹保护仅适用于 Firefox';
  if(!/^[a-zA-Z0-9-]+(?:,[a-zA-Z0-9-]+)*$/.test(draft.preferences.language))return '语言格式示例：en-US,en';
  if(draft.preferences.startupUrl.length>4096)return '启动页面地址过长';
  try {if(draft.preferences.startupUrl!=='about:blank'){const url=new URL(draft.preferences.startupUrl);if(!['http:','https:'].includes(url.protocol)||url.username||url.password)return '启动页面只支持 HTTP、HTTPS 或 about:blank';}}catch{return '请填写完整的启动页面地址';}
  const local=draft.proxy.preProxy;
  if(local){
    if(!['http','https','socks5','vless','vmess'].includes(draft.proxy.mode))return '链式代理需要先配置副本代理';
    if(!['http','https','socks5'].includes(local.mode))return '前置代理协议应为 HTTP、HTTPS 或 SOCKS5';
    let hostValid=local.host.length>0&&local.host.length<=253&&!/[\s\x00-\x1f\x7f/\\@?#\[\]]/.test(local.host);
    if(local.host.includes(':')){try{new URL(`http://[${local.host}]/`);}catch{hostValid=false;}}
    else hostValid=hostValid&&local.host.split('.').every(label=>label.length>0&&label.length<=63&&/^[a-z0-9-]+$/i.test(label));
    if(!hostValid||!Number.isInteger(local.port)||local.port<1||local.port>65535)return '前置代理请填写有效的 IP 或域名及端口，不要包含协议或路径';
  }
  const apiUrl=draft.proxy.apiUrl;
  if(isNodeMode(draft.proxy.mode)){
    if(!draft.proxy.node||!validNode(draft.proxy.node,draft.proxy.mode)||apiUrl!=null||draft.proxy.username)return '节点参数不支持或格式无效，请检查 TLS 和传输设置';
    if(!draft.proxy.host||!Number.isInteger(draft.proxy.port)||draft.proxy.port<1||draft.proxy.port>65535)return '请先导入节点链接';
    if(draft.password&&!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(draft.password))return '节点 UUID 无效，请重新导入节点链接';
  }else if(draft.proxy.node)return '普通代理不能附带节点参数';
  if(apiUrl!=null){
    if(!['http','https','socks5'].includes(draft.proxy.mode))return 'API 提取需要先选择代理协议';
    if(!apiUrl||apiUrl.length>4096||/\s|[\x00-\x1f\x7f]/.test(apiUrl))return '请填写有效的代理 API 地址';
    try{const url=new URL(apiUrl);if(!['http:','https:'].includes(url.protocol)||!url.hostname||url.username||url.password||url.hash)return '代理 API 仅支持不含用户名密码和片段的 HTTP / HTTPS 地址';}catch{return '请填写有效的代理 API 地址';}
  }
  if(['http','https','socks5'].includes(draft.proxy.mode)) {if(apiUrl==null&&(!draft.proxy.host.trim()||/[\s/@?#\\]/.test(draft.proxy.host)))return '请填写 IP 或域名，协议和端口分别填写';if(apiUrl==null&&(!Number.isInteger(draft.proxy.port)||draft.proxy.port<1||draft.proxy.port>65535))return '端口必须在 1–65535 之间';if(new TextEncoder().encode(draft.proxy.username).length>255||/[:\x00-\x1f\x7f]/.test(draft.proxy.username))return '代理用户名不能包含冒号或换行';}
  if(new TextEncoder().encode(draft.password||'').length>255||/[\x00-\x1f\x7f]/.test(draft.password||''))return '密码不能包含控制字符，且最多 255 字节';
  return null;
}
const KEY='claudedone.profiles-demo.v1';
let demo:BrowserProfile[]|null=null;
const name={chrome:'Chrome',edge:'Edge',firefox:'Firefox'};
function data():BrowserProfile[] {
  if(demo)return demo;
  try{const saved=JSON.parse(localStorage.getItem(KEY)||'null');if(Array.isArray(saved)){demo=saved.map(p=>({...p,name:p.legacy&&p.name===`默认 ${name[p.browser as BrowserId]}`?name[p.browser as BrowserId]:p.name,preferences:normalizePreferences(p.preferences),running:false}));return demo!;}}catch{ /* recover demo data */ }
  demo=(['chrome','edge','firefox'] as BrowserId[]).map(browser=>({...defaults(),id:`legacy-${browser}`,name:name[browser],browser,tags:['默认'],createdAt:new Date().toISOString(),updatedAt:new Date().toISOString(),deletedAt:null,lastTest:null,legacy:true,running:false,pendingRestart:false,browserAvailable:true,hasPassword:false,proxyReady:true}));return demo;
}
function saveDemo(){try{localStorage.setItem(KEY,JSON.stringify(data().map(({...p})=>p)));}catch{throw new Error('演示存储已满，无法保存');}}
function get(id:string){const p=data().find(p=>p.id===id);if(!p)throw new Error('副本不存在');return p;}
export async function listProfiles():Promise<BrowserProfile[]> {if(native)return invoke('list_profiles');return structuredClone(data());}
export async function saveProfile(draft:ProfileDraft,id?:string,copyId?:string):Promise<BrowserProfile> {
  const failure=validateDraft(draft);if(failure)throw new Error(failure);
  if(native)return invoke('save_profile',{id:id??null,draft,copyId:copyId??null});
  const clean=structuredClone({...draft,preferences:normalizePreferences(draft.preferences)});delete clean.password;delete clean.proxy.credentialRef;
  if(!id){if(!clean.preferences.fingerprint)clean.preferences.fingerprint=fingerprintDefaults();else if(copyId||data().some(p=>p.preferences.fingerprint?.seed===clean.preferences.fingerprint?.seed))clean.preferences.fingerprint={...clean.preferences.fingerprint,seed:fingerprintDefaults().seed};}
  const p=id?get(id):{...clean,id:crypto.randomUUID().replaceAll('-',''),createdAt:new Date().toISOString(),deletedAt:null,lastTest:null,legacy:false,running:false,pendingRestart:false,browserAvailable:true,hasPassword:false,proxyReady:true,updatedAt:''};
  const running=p.running;const changed=JSON.stringify(p.proxy)!==JSON.stringify(clean.proxy)||JSON.stringify(p.preferences)!==JSON.stringify(clean.preferences);
  const savedPassword=p.hasPassword;
  Object.assign(p,clean,{updatedAt:new Date().toISOString(),lastTest:null,pendingRestart:running&&changed,hasPassword:!draft.clearPassword&&(Boolean(draft.password)||savedPassword||Boolean(copyId&&get(copyId).hasPassword))});
  if(!id)data().push(p);saveDemo();return structuredClone(p);
}
export async function testProxy(draft?:ProfileDraft,id?:string):Promise<ProxyTest> {if(native)return invoke('test_profile_proxy',{draft:draft??null,id:id??null});await new Promise(r=>setTimeout(r,400));const result={ip:'203.0.113.24',country:'SG',latency:328,checkedAt:new Date().toISOString(),connection:'演示结果，未连接真实代理',target:'claude.ai HTTP 200（演示）'};if(id&&!draft){get(id).lastTest=result;saveDemo();}return result;}
export async function matchRegion(draft:ProfileDraft,id?:string):Promise<IpRegion>{if(native)return invoke('match_profile_region',{draft,id:id??null});await new Promise(r=>setTimeout(r,400));return {ip:'203.0.113.24',country:'SG',city:'Singapore',language:'en-SG,en',timezone:'Asia/Singapore',latitude:1.3521,longitude:103.8198,checkedAt:new Date().toISOString()};}
export async function startProfile(id:string){if(native)return invoke<void>('start_profile',{id});get(id).running=true;get(id).pendingRestart=false;saveDemo();}
export async function closeProfile(id:string){if(native)return invoke<void>('close_profile',{id});get(id).running=false;saveDemo();}
export async function restartProfile(id:string){if(native)return invoke<void>('restart_profile',{id});get(id).running=true;get(id).pendingRestart=false;saveDemo();}
export async function deleteProfile(id:string,purge=false){if(native)return invoke<void>('delete_profile',{id,purge});const p=get(id);if(p.running)throw new Error('请先关闭副本');if(purge){if(!p.deletedAt)throw new Error('请先移至最近删除');demo=data().filter(p=>p.id!==id);}else{p.deletedAt=new Date().toISOString();}saveDemo();}
export async function restoreProfile(id:string){if(native)return invoke<void>('restore_profile',{id});get(id).deletedAt=null;saveDemo();}
export async function resetProfile(id:string,resetProxy=false){
 if(native)return invoke<void>('reset_profile',{id,resetProxy});
 const p=get(id);if(p.deletedAt)throw new Error('副本不存在或已删除');if(p.running)throw new Error('请先关闭副本再重置环境');if(p.sharing)throw new Error('收到的共享副本不能修改、复制、转分享或导出');
 p.preferences=defaults().preferences;p.pendingRestart=false;p.lastTest=null;p.lastRegion=null;p.activeProxyEndpoint=null;
 if(resetProxy){p.proxy=defaults().proxy;p.hasPassword=false;}
 p.updatedAt=new Date().toISOString();saveDemo();
}
export async function quitApplication(closeProfiles:boolean){if(native)return invoke<void>('quit_application',{closeProfiles});if(!closeProfiles&&data().some(p=>p.running))throw new Error('仍有运行中的副本');for(const p of data())p.running=false;saveDemo();}

export async function prepareApplicationUpdate(){if(native)return invoke<void>('prepare_application_update');for(const p of data())p.running=false;saveDemo();}
