import type {ProxyConfig} from './browser-profiles';
export interface NodeOptions {tls:'none'|'tls'|'reality';serverName:string;fingerprint:string;publicKey:string;shortId:string;alpn:string[];flow:string;transport:'tcp'|'ws'|'httpupgrade'|'grpc';host:string;path:string;serviceName:string;security:string;alterId:number}
export const isNodeMode=(mode:string)=>mode==='vless'||mode==='vmess';
const fail=()=>new Error('节点链接包含不支持或无效的参数，请检查协议、TLS 和传输设置');
const clean=(value:unknown,max=2048)=>{if(typeof value!=='string'||value.length>max||/[\x00-\x1f\x7f]/.test(value))throw fail();return value;};
const text=(value:unknown,fallback='')=>value==null?fallback:clean(value);
const decode64=(value:string)=>{try{const bytes=Uint8Array.from(atob(value.replace(/-/g,'+').replace(/_/g,'/')),c=>c.charCodeAt(0));return new TextDecoder('utf-8',{fatal:true}).decode(bytes);}catch{throw fail();}};
export function validNode(node:NodeOptions,mode:string):boolean {
 try {
  if(!isNodeMode(mode)||!node||!['none','tls','reality'].includes(node.tls)||!['tcp','ws','httpupgrade','grpc'].includes(node.transport))return false;
  for(const key of ['serverName','fingerprint','publicKey','shortId','flow','host','path','serviceName','security'] as const)clean(node[key]);
  if(!['','chrome','firefox','safari','edge','ios','android','random','randomized'].includes(node.fingerprint))return false;
  if(/[\s/@?#\\]/.test(node.serverName)||/\s/.test(node.host))return false;
  if(!Array.isArray(node.alpn)||node.alpn.length>8||node.alpn.some(v=>typeof v!=='string'||!v||v.length>64||/[\s\x00-\x1f\x7f]/.test(v)))return false;
  if(!Number.isInteger(node.alterId)||node.alterId<0||node.alterId>65535||!['auto','aes-128-gcm','chacha20-poly1305','none','zero'].includes(node.security))return false;
  if(node.tls==='none'&&(node.serverName||node.fingerprint||node.alpn.length))return false;
  if(node.tls==='reality'&&(mode!=='vless'||!node.serverName||!node.fingerprint||!/^[A-Za-z0-9_-]{43}$/.test(node.publicKey)||node.shortId.length>16||node.shortId.length%2||!/^[0-9a-f]*$/i.test(node.shortId)))return false;
  if(node.tls!=='reality'&&(node.publicKey||node.shortId))return false;
  if(node.flow&&(mode!=='vless'||node.flow!=='xtls-rprx-vision'||node.tls==='none'||node.transport!=='tcp'))return false;
  if(mode==='vless'&&(node.alterId||node.security!=='auto'))return false;
  if(node.transport==='tcp'&&(node.host||node.path||node.serviceName))return false;
  if(node.transport==='grpc'&&(node.host||node.path)||node.transport!=='grpc'&&node.serviceName)return false;
  return true;
 }catch{return false;}
}
export function parseNodeUri(input:string):{proxy:ProxyConfig;password:string} {
 try {
  const value=clean(input.trim(),16384);
  const node:NodeOptions={tls:'none',serverName:'',fingerprint:'',publicKey:'',shortId:'',alpn:[],flow:'',transport:'tcp',host:'',path:'',serviceName:'',security:'auto',alterId:0};
  let host:string,port:number,uuid:string,mode:'vless'|'vmess';
  if(value.startsWith('vless://')) {
   mode='vless';const url=new URL(value);host=url.hostname.replace(/^\[|\]$/g,'');port=Number(url.port);uuid=decodeURIComponent(url.username);
   if(url.password||url.pathname&&url.pathname!=='/')throw fail();
   const allowed=['security','sni','serverName','fp','pbk','sid','alpn','flow','type','host','path','serviceName','encryption','headerType'];
   const seen=new Set<string>();url.searchParams.forEach((_,key)=>{if(!allowed.includes(key)||seen.has(key))throw fail();seen.add(key);});
   const get=(key:string,fallback='')=>url.searchParams.get(key)??fallback;
   if(get('encryption','none')!=='none'||get('headerType','none')!=='none'||get('sni')&&get('serverName')&&get('sni')!==get('serverName'))throw fail();
   Object.assign(node,{tls:get('security','none'),serverName:get('sni',get('serverName')),fingerprint:get('fp'),publicKey:get('pbk'),shortId:get('sid'),alpn:get('alpn')?get('alpn').split(','):[],flow:get('flow'),transport:get('type','tcp'),host:get('host'),path:get('path'),serviceName:get('serviceName')});
  }else if(value.startsWith('vmess://')){
   mode='vmess';const encoded=value.slice(8);if(/[?#\s]/.test(encoded))throw fail();
   const data=JSON.parse(decode64(encoded));if(!data||Array.isArray(data)||typeof data!=='object')throw fail();
   const allowed=['v','ps','add','port','id','aid','scy','net','type','host','path','tls','sni','alpn','fp'];
   if(Object.keys(data).some(key=>!allowed.includes(key))||!['','none'].includes(text(data.type))||!['','2'].includes(String(data.v??'')))throw fail();
   host=text(data.add);port=Number(data.port);uuid=text(data.id);
   const transport=text(data.net,'tcp');
   if(transport==='grpc'&&text(data.host))throw fail();
   Object.assign(node,{tls:text(data.tls)||'none',serverName:text(data.sni),fingerprint:text(data.fp),alpn:text(data.alpn)?text(data.alpn).split(','):[],transport,host:transport==='grpc'?'':text(data.host),path:transport==='grpc'?'':text(data.path),serviceName:transport==='grpc'?text(data.path):'',security:text(data.scy,'auto'),alterId:Number(data.aid??0)});
  }else throw fail();
  if(!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(uuid)||!Number.isInteger(port)||port<1||port>65535||!host||host.length>253||/[\s/@?#\\]/.test(host)||!validNode(node,mode))throw fail();
  if(host.includes(':'))new URL(`http://[${host}]/`);else if(!host.split('.').every(p=>p.length>0&&p.length<=63&&/^[a-z0-9-]+$/i.test(p)))throw fail();
  return {proxy:{mode,host,port,username:'',node},password:uuid.toLowerCase()};
 }catch{throw fail();}
}
