import {expect,it} from 'vitest';
import {parseProxyUri,defaults,validateDraft,selectProxyMode} from './browser-profiles';
const uuid='11111111-1111-4111-8111-111111111111';
it('imports VLESS Reality and VMess WS without storing raw links or UUID in config',()=>{
 const vless=parseProxyUri(`vless://${uuid}@node.example:443?security=reality&sni=edge.example&pbk=${'A'.repeat(43)}&sid=01&fp=chrome&flow=xtls-rprx-vision&type=tcp#JP`);
 expect(vless.password).toBe(uuid);
 expect(vless.proxy).toMatchObject({mode:'vless',host:'node.example',port:443,node:{tls:'reality',serverName:'edge.example',flow:'xtls-rprx-vision'}});
 expect(JSON.stringify(vless.proxy)).not.toContain(uuid);
 const vmess=parseProxyUri('vmess://'+btoa(JSON.stringify({v:'2',ps:'test',add:'node.example',port:'443',id:uuid,aid:'0',scy:'auto',net:'ws',type:'none',host:'cdn.example',path:'/socket',tls:'tls',sni:'cdn.example'})));
 expect(vmess).toMatchObject({password:uuid,proxy:{mode:'vmess',node:{transport:'ws',path:'/socket',host:'cdn.example',tls:'tls'}}});
 expect(validateDraft({...defaults(),name:'node',...vmess})).toBeNull();
});
it('rejects unsupported or ambiguous links instead of silently weakening them',()=>{
 for(const suffix of ['security=tls&allowInsecure=1','type=xhttp','type=tcp&type=ws','encryption=unsupported','security=reality','type=tcp&headerType=http','security=tls&unknown=1']) {
  const uri=`vless://${uuid}@node.example:443?${suffix}`;let error:unknown;
  try{parseProxyUri(uri);}catch(e){error=e;}
  expect(error).toBeTruthy();expect(String(error)).not.toContain(uuid);expect(String(error)).not.toContain(uri);
 }
 expect(()=>parseProxyUri('vmess://'+btoa(JSON.stringify({v:'2',add:'node.example',port:'443',id:uuid,net:'grpc',host:'custom-authority.example',path:'service',tls:'tls'})))).toThrow();
});

it('clears transient node credentials when switching protocol families',()=>{
 const node={...defaults(),...parseProxyUri(`vless://${uuid}@node.example:443`)};
 expect(selectProxyMode(node,'http').password).toBe('');
 expect(selectProxyMode(node,'vless').password).toBe(uuid);
 const ordinary={...defaults(),password:'ordinary-secret',proxy:{mode:'http' as const,host:'proxy.example',port:8080,username:'user'}};
 expect(selectProxyMode(ordinary,'vmess').password).toBe('');
 expect(selectProxyMode(ordinary,'vmess').proxy.username).toBe('');
 expect(selectProxyMode(ordinary,'https').password).toBe('ordinary-secret');
});
