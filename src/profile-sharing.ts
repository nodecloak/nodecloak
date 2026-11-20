import {invoke} from '@tauri-apps/api/core';
import {native} from './bridge';
import type {BrowserId} from './domain';
export type ShareStatus='pending'|'accepted'|'declined'|'revoked'|'left'|'expired';
export interface ProfileShare {id:string;teamId?:string;sourceProfileId:string;profileName:string;browser:BrowserId;senderId:string;senderName:string;senderEmail:string;recipientId:string;recipientName:string;recipientEmail:string;includesSession:boolean;note:string;status:ShareStatus;createdAt:string;expiresAt:string;acceptedAt:string|null;localProfileId?:string;localDataPresent?:boolean;deviceAccess?:'ready'|'authorizationRequired'|'unavailable'}
export interface SharingList {sent:ProfileShare[];received:ProfileShare[];serverTime:string}
export interface SharedProfileGrant {id:string;senderName:string;expiresAt:string;status:ShareStatus}
export type SharingAction={type:'list'}|{type:'create';profileId:string;name:string;email:string;expiresAt:string;includesSession:boolean;note:string;team?:{accountId:string;teamId:string;memberId:string}}|{type:'accept';id:string;authorizeDevice?:boolean}|{type:'decline'|'revoke'|'leave'|'cleanup';id:string}|{type:'extend';id:string;expiresAt:string};
export async function sharingAction<T=ProfileShare>(action:SharingAction):Promise<T>{if(!native)throw new Error('请在桌面客户端使用副本分享');const result=await invoke<T|{share:ProfileShare;localProfileId?:string}>('sharing_action',{action});if(result&&typeof result==='object'&&'share' in result)return {...result.share,...(result.localProfileId?{localProfileId:result.localProfileId}:{})} as T;return result as T;}
export const shareLabels:Record<ShareStatus,string>={pending:'待接受',accepted:'已接受',declined:'已拒绝',revoked:'已撤销',left:'已退出',expired:'已过期'};
export function effectiveShareStatus(share:{status:ShareStatus;expiresAt:string},now=Date.now()):ShareStatus{return ['pending','accepted'].includes(share.status)&&Date.parse(share.expiresAt)<=now?'expired':share.status;}
export function shareDeadline(period:string,custom:string,now=Date.now()):string {const date=period==='custom'?new Date(custom):new Date(now+Number(period)*86400000);if(!Number.isFinite(date.getTime())||date.getTime()<=now)throw new Error('截止时间必须晚于当前时间');if(date.getTime()-now>366*86400000)throw new Error('有效期最多为 366 天');return date.toISOString();}
export function localDeadline(iso:string):string {const date=new Date(iso);return new Date(date.getTime()-date.getTimezoneOffset()*60000).toISOString().slice(0,16);}
const sharingErrors:Record<string,string>={
 recipient_device_unavailable:'对方需要先登录 NodeCloak，再重新发送邀请。',
 recipient_device_approval_required:'对方还没有可接收分享的设备。请让对方登录 NodeCloak 后重试；接受邀请时可直接授权设备。',
 recipient_not_found:'未找到与用户名和邮箱匹配的其他账号。请核对对方的个人资料名称和邮箱。',
 recipient_device_changed:'对方的设备授权已变化，请重新发送邀请。',
 device_approval_required:'此设备尚未获得该分享的授权。点击“接受分享”，在确认窗口中授权当前设备后继续。',
 share_authorization_required:'此设备尚未获得该分享的授权。点击“接受分享”，在确认窗口中授权当前设备后继续。',
 share_device_unavailable:'发送邀请时未包含此设备，或设备身份已变化。请让朋友重新发送邀请。',
 invalid_deadline:'截止时间无效或已过期，请重新设置有效期。',
 share_expired:'这个分享已过期，请让朋友发送新的邀请。',
 share_inactive:'这个分享已结束，无法继续使用。需要继续使用时，请让朋友重新分享。',
 share_quota_exceeded:'分享数量或数据大小已达上限。请撤销不再使用的分享，或减少分享数据后重试。',
 share_id_conflict:'这次邀请未能创建，请重新发送。',
 team_assignment_exists:'此成员已有该副本的有效分配。',
 team_member_inactive:'该成员尚未加入或已退出团队，请刷新后重试。',
 team_not_found:'此团队不存在或你没有操作权限。',
 unauthorized:'登录已失效，请重新登录后重试。',
};
export function shareErrorMessage(error:unknown):string {
 const value=error instanceof Error?error.message:String(error);
 const code=/^(?:Error: )?云端请求失败 \(\d{3}\): ([a-z_]+)$/.exec(value)?.[1];
 return code&&sharingErrors[code]||value;
}
