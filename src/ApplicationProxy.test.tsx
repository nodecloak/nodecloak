// @vitest-environment jsdom
import {act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {beforeEach,afterEach,it,expect,vi} from 'vitest';
import ApplicationProxy from './ApplicationProxy';
import type {AppProxyView} from './application-proxy';
import {setLanguage} from './i18n';
const api=vi.hoisted(()=>({call:vi.fn(),choose:vi.fn(),test:vi.fn()}));
vi.mock('./application-proxy',()=>({applicationProxy:api.call,chooseProxyExecutable:api.choose,testProxyDestination:api.test}));
let host:HTMLDivElement,root:Root,view:AppProxyView;
beforeEach(()=>{
 localStorage.clear();setLanguage('zh');Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
 view={supported:true,settings:{version:1,proxies:[],rules:[]},apps:[],discoveryChecked:true,runtime:{phase:'off',interface:null,since:null,error:null,needsCleanup:false,connections:[],events:[],warnings:[]}};
 api.call.mockImplementation(async()=>structuredClone(view));host=document.createElement('div');document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.resetAllMocks();});
const button=(text:string)=>Array.from(host.querySelectorAll<HTMLButtonElement>('button')).find(b=>b.textContent?.includes(text));
it('offers Mac application routing with Mac administrator consent rather than Windows instructions',async()=>{
 Object.assign(view,{platform:'macos'});
 await act(async()=>root.render(<ApplicationProxy/>));
 expect(button('开启应用代理')!.disabled).toBe(false);
 expect(host.textContent).toContain('macOS 管理员授权');
 expect(host.textContent).not.toContain('Windows 管理员授权');
 expect(host.textContent).not.toContain('仅支持 Windows');
});
it('opens inactive and never starts routing merely by mounting the page',async()=>{
 await act(async()=>root.render(<ApplicationProxy/>));
 expect(button('开启应用代理')).toBeDefined();expect(host.textContent).toContain('未开启');
 expect(api.call.mock.calls.some(([action])=>action==='start')).toBe(false);
});
it('requires an explicit start and a confirmed running result before showing stop',async()=>{
 api.call.mockImplementation(async(action:string)=>{if(action==='start')view.runtime.phase='running';if(action==='stop')view.runtime.phase='off';return structuredClone(view);});
 await act(async()=>root.render(<ApplicationProxy/>));await act(async()=>button('开启应用代理')!.click());
 expect(button('停止应用代理')).toBeDefined();await act(async()=>button('停止应用代理')!.click());expect(button('开启应用代理')).toBeDefined();
});
it('cancelled administrator authorization retains off state and an actionable error',async()=>{
 api.call.mockImplementation(async(action:string)=>{if(action==='start')throw new Error('已取消 Windows 管理员授权');return structuredClone(view);});
 await act(async()=>root.render(<ApplicationProxy/>));await act(async()=>button('开启应用代理')!.click());
 expect(host.querySelector('[role="alert"]')?.textContent).toContain('已取消 Windows 管理员授权');expect(button('开启应用代理')!.disabled).toBe(false);
});
it('unsupported platforms never offer an enabled start control',async()=>{
 view.supported=false;await act(async()=>root.render(<ApplicationProxy/>));
 expect(host.textContent).toContain('当前系统仍可使用浏览器副本代理');expect(button('开启应用代理')!.disabled).toBe(true);
});
it('dismisses a persisted runtime error without changing routing state',async()=>{
 view.runtime.phase='error';view.runtime.error='启动已取消';await act(async()=>root.render(<ApplicationProxy/>));
 expect(host.querySelector('[role="alert"]')).not.toBeNull();
 await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="关闭提示"]')!.click());
 expect(host.querySelector('[role="alert"]')).toBeNull();expect(api.call.mock.calls.some(([a])=>a==='start')).toBe(false);
});
it('can cancel a pending start without accepting its late running response',async()=>{
 let finish!:(value:AppProxyView)=>void;
 api.call.mockImplementation(async(action:string)=>{if(action==='start')return new Promise<AppProxyView>(resolve=>{finish=resolve;});return structuredClone(view);});
 await act(async()=>root.render(<ApplicationProxy/>));await act(async()=>button('开启应用代理')!.click());
 await act(async()=>button('取消启动')!.click());
 expect(api.call.mock.calls.some(([a])=>a==='stop')).toBe(true);
 const late=structuredClone(view);late.runtime.phase='running';await act(async()=>finish(late));
 expect(button('开启应用代理')).toBeDefined();expect(button('停止应用代理')).toBeUndefined();
});
it('selecting a destination saves only that application rule and does not start TUN',async()=>{
 const id='0b4fcb93-ec83-4f9b-996a-f89f7aa96025';
 view.settings.proxies=[{id,name:'Local test',hasPassword:false,proxy:{mode:'socks5',host:'127.0.0.1',port:1080,username:''}}];
 view.settings.rules=[{id:'app-rule',app:'codex',executable:'',proxyId:null,enabled:false}];view.apps=[{id:'codex',name:'Codex',paths:['C:\\Apps\\Codex.exe'],running:1}];
 await act(async()=>root.render(<ApplicationProxy/>));const select=host.querySelector('select')!;
 await act(async()=>{select.value=id;select.dispatchEvent(new Event('change',{bubbles:true}));});
 expect(api.call).toHaveBeenCalledWith('saveRules',{rules:[{...view.settings.rules[0],proxyId:id,enabled:true}]});
 expect(api.call.mock.calls.some(([a])=>a==='start')).toBe(false);
});
it('refreshes native cleanup state after a failed start so the user can stop it',async()=>{
 api.call.mockImplementation(async(action:string)=>{if(action==='start'){view.runtime.phase='error';view.runtime.needsCleanup=true;view.runtime.error='接口正在释放';throw new Error('接口正在释放');}return structuredClone(view);});
 await act(async()=>root.render(<ApplicationProxy/>));await act(async()=>button('开启应用代理')!.click());
 expect(button('停止应用代理')).toBeDefined();
});
