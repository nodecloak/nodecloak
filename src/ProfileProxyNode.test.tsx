// @vitest-environment jsdom
import {act,useState} from 'react';
import {createRoot} from 'react-dom/client';
import {expect,it} from 'vitest';
import {defaults,type ProfileDraft} from './browser-profiles';
import ProfileProxyNode from './ProfileProxyNode';
import {getLanguage,setLanguage} from './i18n';
it('imports HY2 userpass with a masked auth field and visible certificate policy',async()=>{
 const previousLanguage=getLanguage();setLanguage('zh');
 Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
 let draft:ProfileDraft=defaults();
 function Harness(){const [value,set]=useState(draft);return <ProfileProxyNode draft={value} onChange={next=>{draft=next;set(next);}}/>;}
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 try{
  await act(async()=>root.render(<Harness/>));
  const input=host.querySelector<HTMLInputElement>('#profile-node-link')!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,'hy2://user:SecretAbC@node.example:443?insecure=1&sni=example.com');input.dispatchEvent(new Event('input',{bubbles:true}));});
  await act(async()=>host.querySelector('button')!.click());
  expect(draft.proxy.mode).toBe('hysteria2');expect(input.value).toBe('');
  expect(host.querySelector<HTMLInputElement>('[aria-label="节点认证信息"]')?.type).toBe('password');
  expect(host.textContent).toContain('QUIC / UDP');expect(host.textContent).toContain('跳过证书验证');
  expect(host.textContent).not.toContain('SecretAbC');
 }finally{await act(async()=>root.unmount());host.remove();setLanguage(previousLanguage);}
});
it('imports a masked node link, clears it and retains the first-hop proxy',async()=>{
 Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
 let draft:ProfileDraft={...defaults(),proxy:{mode:'vless',host:'',port:443,username:'',preProxy:{mode:'http',host:'192.168.10.1',port:7890}}};
 function Harness(){const [value,set]=useState(draft);return <ProfileProxyNode draft={value} onChange={next=>{draft=next;set(next);}}/>;}
 const host=document.createElement('div');document.body.append(host);const root=createRoot(host);
 try{
  await act(async()=>root.render(<Harness/>));
  const input=host.querySelector<HTMLInputElement>('#profile-node-link')!;expect(input.type).toBe('password');
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value')!.set!.call(input,'vless://11111111-1111-4111-8111-111111111111@node.example:443?security=tls');input.dispatchEvent(new Event('input',{bubbles:true}));});
  await act(async()=>host.querySelector('button')!.click());
  expect(input.value).toBe('');expect(draft.proxy.preProxy?.host).toBe('192.168.10.1');expect(draft.proxy.node?.tls).toBe('tls');
  expect(host.textContent).toContain('node.example:443');expect(host.textContent).not.toContain('11111111');
 }finally{await act(async()=>root.unmount());host.remove();}
});
