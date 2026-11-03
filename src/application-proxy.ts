import type {ProxyConfig,ProxyTest} from './browser-profiles';
import {invoke} from '@tauri-apps/api/core';
import {native} from './bridge';
export interface AppProxyRule {id:string;app:'codex'|'chatgpt'|'claude'|'custom';executable:string;proxyId:string|null;enabled:boolean}
export interface AppProxyDestination {id:string;name:string;proxy:ProxyConfig;hasPassword:boolean}
export interface ProxyApplication {id:string;name:string;paths:string[];running:number}
export interface AppProxyConnection {id:string;app:string;processPath:string;destination:string;network:string;proxyId:string;upload:number;download:number;seenAt:string}
export interface AppProxyEvent {at:string;event:string;message:string}
export interface AppProxyView {supported:boolean;simulation?:boolean;settings:{version:number;proxies:AppProxyDestination[];rules:AppProxyRule[]};apps:ProxyApplication[];discoveryChecked:boolean;runtime:{phase:'off'|'starting'|'running'|'stopping'|'error';interface:string|null;since:string|null;error:string|null;needsCleanup:boolean;connections:AppProxyConnection[];events:AppProxyEvent[];warnings:string[]}}
const apps=[{id:'codex',name:'Codex'},{id:'chatgpt',name:'ChatGPT'},{id:'claude',name:'Claude'}];
const demo:AppProxyView={supported:true,simulation:true,settings:{version:1,proxies:[],rules:apps.map((a,i)=>({id:`demo-${i}`,app:a.id as AppProxyRule['app'],executable:'',proxyId:null,enabled:false}))},apps:apps.map(a=>({...a,paths:[`C:\\Demo\\${a.name}\\${a.id==='codex'?'ChatGPT':a.name}.exe`],running:0})),discoveryChecked:true,runtime:{phase:'off',interface:null,since:null,error:null,needsCleanup:false,connections:[],events:[],warnings:[]}};
export async function applicationProxy(action:string,payload?:unknown):Promise<AppProxyView>{
 if(native)return invoke('application_proxy_action',{action,payload:payload??null});
 const data=payload as Record<string,unknown>|undefined;
 if(action==='start'){
  if(!demo.settings.rules.some(r=>r.enabled&&r.proxyId))throw new Error('请至少为一个已安装应用选择代理节点');
  demo.runtime.phase='running';demo.runtime.interface='演示接口';demo.runtime.since=new Date().toISOString();
 }else if(action==='stop'){demo.runtime.phase='off';demo.runtime.interface=null;demo.runtime.connections=[];}
 else if(action==='saveRules')demo.settings.rules=structuredClone(data?.rules as AppProxyRule[]);
 else if(action==='saveNode'){
  const draft=data as unknown as {id?:string;name:string;proxy:ProxyConfig;password?:string;clearPassword?:boolean};const id=draft.id??crypto.randomUUID();
  const old=demo.settings.proxies.find(p=>p.id===id);const node={id,name:draft.name,proxy:structuredClone(draft.proxy),hasPassword:!!draft.password||(!draft.clearPassword&&!!old?.hasPassword)};
  demo.settings.proxies=demo.settings.proxies.filter(p=>p.id!==id).concat(node);
 }else if(action==='deleteNode'){
  demo.settings.proxies=demo.settings.proxies.filter(p=>p.id!==data?.id);
  demo.settings.rules=demo.settings.rules.map(r=>r.proxyId===data?.id?{...r,proxyId:null,enabled:false}:r);
 }
 return structuredClone(demo);
}
export async function chooseProxyExecutable():Promise<string|null>{return native?invoke('application_proxy_action',{action:'pickExecutable',payload:null}):'C:\\Demo\\Custom\\Example.exe';}
export async function testProxyDestination(id:string):Promise<ProxyTest>{
 if(native)return invoke('application_proxy_action',{action:'testNode',payload:{id}});
 return {ip:null,country:null,latency:0,checkedAt:new Date().toISOString(),connection:'演示结果，未连接实际代理',target:'演示'};
}
