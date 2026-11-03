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
 expect(host.textContent).toContain('仅支持 Windows');expect(button('开启应用代理')!.disabled).toBe(true);
});
it('dismisses a persisted runtime error without changing routing state',async()=>{
 view.runtime.phase='error';view.runtime.error='启动已取消';await act(async()=>root.render(<ApplicationProxy/>));
 expect(host.querySelector('[role="alert"]')).not.toBeNull();
 await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="关闭提示"]')!.click());
 expect(host.querySelector('[role="alert"]')).toBeNull();expect(api.call.mock.calls.some(([a])=>a==='start')).toBe(false);
});
