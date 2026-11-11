import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { invoke } from '@tauri-apps/api/core';
import { version } from '../package.json';

export interface UpdateCheck { currentVersion: string; latestVersion: string; available: boolean; supported: boolean; notes: string[]; publishedAt: string; checkedAt: string; portable?:boolean; manualDownload?:boolean }
export interface UpdateProgress { phase: 'downloading' | 'verifying' | 'installing' | 'ready'; downloaded: number; total?: number }
let pending: Update | null = null;
let installing = false;
let checking: Promise<UpdateCheck> | null = null;

export function checkNativeUpdate(): Promise<UpdateCheck> {
  if(installing) return Promise.reject(new Error('正在安装更新，请稍候。'));
  if(checking) return checking;
  checking=(async()=>{
    const previous=pending;
    pending=null;
    if(previous) await previous.close();
    if(!await invoke<boolean>('supports_in_app_updates')) {
      return {currentVersion:version,latestVersion:version,available:false,supported:false,manualDownload:true,notes:[],publishedAt:'',checkedAt:new Date().toISOString()};
    }
    pending=await check({timeout:15000,headers:{'Cache-Control':'no-cache'}});
    return {currentVersion:version,latestVersion:pending?.version ?? version,available:pending!==null,supported:true,notes:pending?.body?.split(/\r?\n/).map(line=>line.replace(/^[-*]\s+/, '').trim()).filter(Boolean) ?? [],publishedAt:pending?.date ?? '',checkedAt:new Date().toISOString()};
  })().finally(()=>{checking=null;});
  return checking;
}

export async function installNativeUpdate(onProgress:(progress:UpdateProgress)=>void):Promise<void> {
  if(installing || checking) throw new Error('更新操作正在进行，请稍候。');
  if(!pending && (await checkNativeUpdate()).manualDownload) throw new Error('当前安装方式请从官网下载并安装新版。副本和设置会保留。');
  if(!pending) throw new Error('没有可安装的新版本，请重新检查更新。');
  installing=true;
  const update=pending;
  let downloaded=0;
  let total:number|undefined;
  try {
    onProgress({phase:'downloading',downloaded});
    await update.download(event=>{
      if(event.event==='Started') { downloaded=0; total=event.data.contentLength; }
      if(event.event==='Progress') downloaded+=event.data.chunkLength;
      onProgress({phase:event.event==='Finished'?'verifying':'downloading',downloaded,total});
    },{timeout:180000});
    // download() resolves only after the updater verifies the package signature.
    onProgress({phase:'installing',downloaded,total});
    await update.install({restartAfterInstall:true});
    onProgress({phase:'ready',downloaded,total});
  } finally {
    pending=null;
    installing=false;
    await update.close().catch(()=>{});
  }
}

export const restartUpdatedApp=()=>relaunch();
