import {invoke} from '@tauri-apps/api/core';

export interface TeamMember {
  id: string;
  name: string;
  email: string;
  status: 'pending' | 'active';
}

export interface Team {
  id: string;
  name: string;
  ownerName: string;
  role: 'owner' | 'member';
  status: 'pending' | 'active';
  membershipId?: string;
  members: TeamMember[];
}

export type TeamAction =
  | {type: 'list'}
  | {type: 'create'; name: string}
  | {type: 'invite'; teamId: string; name: string; email: string}
  | {type: 'accept' | 'remove'; teamId: string; memberId: string};

export function teamAction<T>(accountId: string, action: TeamAction): Promise<T> {
  return invoke<T>('team_action', {accountId, action});
}

const errors: Record<string, string> = {
  invalid_team_name: '团队名称需要包含 1 到 60 个字符。',
  team_limit: '已达到可创建的团队数量上限。',
  team_member_limit: '这个团队的成员和邀请数量已达上限。',
  team_member_exists: '此账号已加入团队或已有待接受的邀请。',
  team_member_not_found: '成员或邀请已变化，请刷新团队。',
  team_member_inactive: '该成员尚未加入或已退出团队，请刷新后重试。',
  team_not_found: '此团队不存在或你没有操作权限。',
  team_assignment_exists: '此成员已有该副本的有效分配。',
  invalid_recipient: '请填写有效的成员名称和邮箱。',
  recipient_not_found: '未找到匹配的账号，请确认名称和邮箱。',
  try_later: '邀请过于频繁，请稍后重试。',
};

export function teamError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  const code = /云端请求失败 \(\d{3}\): ([a-z_]+)$/.exec(message)?.[1];
  return code && errors[code] || message;
}
