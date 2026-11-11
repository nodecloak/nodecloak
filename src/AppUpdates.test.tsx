// @vitest-environment jsdom
import {act} from 'react';
import {createRoot,type Root} from 'react-dom/client';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import AppUpdates from './AppUpdates';
import {setLanguage} from './i18n';

const api=vi.hoisted(()=>({check:vi.fn(),download:vi.fn(),install:vi.fn(),prepare:vi.fn()}));
vi.mock('./bridge',()=>({native:true,checkForUpdates:api.check,openDownloadPage:api.download,installAvailableUpdate:api.install,restartAfterUpdate:vi.fn()}));
vi.mock('./browser-profiles',()=>({listProfiles:vi.fn(),prepareApplicationUpdate:api.prepare}));
let host:HTMLDivElement,root:Root;
beforeEach(()=>{
  localStorage.clear();setLanguage('en');
  Object.assign(globalThis,{IS_REACT_ACT_ENVIRONMENT:true});
  HTMLDialogElement.prototype.close=function(){this.open=false;};
  api.check.mockResolvedValue({currentVersion:'5.0.2',latestVersion:'5.0.2',available:false,supported:false,manualDownload:true,notes:[],publishedAt:'',checkedAt:new Date().toISOString()});
  api.download.mockResolvedValue(undefined);
  host=document.createElement('div');document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.clearAllMocks();});

it('offers the download page for deb without suggesting that the installed version is latest',async()=>{
  await act(async()=>root.render(<AppUpdates expanded/>));
  expect(host.textContent).toContain('This installation uses manual updates');
  expect(host.textContent).not.toContain('当前已是最新版本');
  expect(host.querySelector('.update-last-check')).toBeNull();
  expect(host.querySelector('input[role="switch"]')).toBeNull();
  const download=Array.from(host.querySelectorAll('button')).find(button=>button.textContent==='Open official downloads');
  expect(download).toBeDefined();
  await act(async()=>download!.click());
  expect(api.download).toHaveBeenCalledOnce();
  expect(api.install).not.toHaveBeenCalled();
  expect(api.prepare).not.toHaveBeenCalled();
});

it('reports a failed manual-download page launch',async()=>{
  api.download.mockRejectedValueOnce(new Error('Cannot open browser'));
  await act(async()=>root.render(<AppUpdates expanded/>));
  const download=Array.from(host.querySelectorAll('button')).find(button=>button.textContent==='Open official downloads')!;
  await act(async()=>download.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toContain('Cannot open browser');
});
