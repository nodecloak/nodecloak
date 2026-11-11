import {beforeEach,describe,it,expect,vi} from 'vitest';
const mocks=vi.hoisted(()=>({check:vi.fn(),relaunch:vi.fn(),invoke:vi.fn()}));
vi.mock('@tauri-apps/api/core',()=>({invoke:mocks.invoke}));
vi.mock('@tauri-apps/plugin-updater',()=>({check:mocks.check}));
vi.mock('@tauri-apps/plugin-process',()=>({relaunch:mocks.relaunch}));
beforeEach(()=>{vi.resetModules();vi.clearAllMocks();mocks.invoke.mockResolvedValue(true);});
function update() {
  return {version:'0.5.10',body:'- 支持自动更新',date:'2026-10-07T00:00:00Z',close:vi.fn().mockResolvedValue(undefined),download:vi.fn().mockResolvedValue(undefined),install:vi.fn().mockResolvedValue(undefined)};
}
describe('in-app updater',()=>{
  it('routes deb installs to manual download without checking or installing an AppImage',async()=>{
    mocks.invoke.mockResolvedValue(false);
    const service=await import('./app-updater');
    const result=await service.checkNativeUpdate();
    expect(result).toMatchObject({available:false,supported:false,manualDownload:true});
    expect(mocks.invoke).toHaveBeenCalledWith('supports_in_app_updates');
    expect(mocks.check).not.toHaveBeenCalled();
    await expect(service.installNativeUpdate(vi.fn())).rejects.toThrow('从官网下载');
    expect(mocks.check).not.toHaveBeenCalled();
  });
  it('does not attempt an update if package support cannot be determined',async()=>{
    mocks.invoke.mockRejectedValue(new Error('Capability unavailable'));
    const service=await import('./app-updater');
    await expect(service.checkNativeUpdate()).rejects.toThrow('Capability unavailable');
    expect(mocks.check).not.toHaveBeenCalled();
  });
  it('closes stale resources and reports no update without installing',async()=>{
    const next=update();mocks.check.mockResolvedValueOnce(next).mockResolvedValueOnce(null);
    const service=await import('./app-updater');
    expect((await service.checkNativeUpdate()).available).toBe(true);
    expect((await service.checkNativeUpdate()).available).toBe(false);
    expect(next.close).toHaveBeenCalledOnce();
    expect(next.install).not.toHaveBeenCalled();
  });
  it('never installs when download or signature verification fails',async()=>{
    const next=update();next.download.mockRejectedValue(new Error('invalid signature'));mocks.check.mockResolvedValue(next);
    const service=await import('./app-updater');await service.checkNativeUpdate();
    const progress=vi.fn();
    await expect(service.installNativeUpdate(progress)).rejects.toThrow('invalid signature');
    expect(next.install).not.toHaveBeenCalled();expect(next.close).toHaveBeenCalledOnce();
    expect(progress.mock.calls.some(([value])=>value.phase==='ready')).toBe(false);
  });
  it('blocks concurrent checks while updating and installs only after verified download',async()=>{
    const next=update();mocks.check.mockResolvedValue(next);
    let finish:()=>void=()=>{};
    next.download.mockImplementation(()=>new Promise<void>(resolve=>{finish=resolve;}));
    const service=await import('./app-updater');await service.checkNativeUpdate();
    const progress=vi.fn();const install=service.installNativeUpdate(progress);
    await expect(service.checkNativeUpdate()).rejects.toThrow('正在安装');
    await expect(service.installNativeUpdate(progress)).rejects.toThrow('正在进行');
    expect(next.install).not.toHaveBeenCalled();finish();await install;
    expect(next.install).toHaveBeenCalledWith({restartAfterInstall:true});
    expect(progress.mock.calls.at(-1)?.[0].phase).toBe('ready');
  });
});
