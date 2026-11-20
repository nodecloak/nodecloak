// @vitest-environment jsdom
import {act} from 'react';
import {createRoot, type Root} from 'react-dom/client';
import {afterEach, beforeEach, expect, it, vi} from 'vitest';
import type {Controller} from './AccountWorkspace';
import type {SharingController} from './ProfileSharing';
import {setLanguage} from './i18n';
import TeamWorkspace from './TeamWorkspace';
import type {Team, TeamAction} from './teams';

const api = vi.hoisted(() => ({invoke: vi.fn(), profiles: vi.fn()}));
vi.mock('@tauri-apps/api/core', () => ({invoke: api.invoke}));
vi.mock('./bridge', () => ({native: true}));
vi.mock('./browser-profiles', () => ({listProfiles: api.profiles}));

const owner: Team = {
  id: 'team', name: 'Support', ownerName: 'Maya', role: 'owner', status: 'active',
  members: [
    {id: 'active', name: 'Alex', email: 'alex@example.test', status: 'active'},
    {id: 'pending', name: 'Jo', email: 'jo@example.test', status: 'pending'},
  ],
};
const sharing: SharingController = {
  data: {sent: [], received: [], serverTime: ''}, loading: false, busy: false,
  notice: '', error: '', refresh: vi.fn(), act: vi.fn(), clearError: vi.fn(),
};
const cloudFor = (id: string) => ({status: {account: {id}}} as Controller);
let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  Object.assign(globalThis, {IS_REACT_ACT_ENVIRONMENT: true});
  Object.assign(HTMLDialogElement.prototype, {showModal: vi.fn(), close: vi.fn()});
  Object.assign(HTMLElement.prototype, {scrollIntoView: vi.fn()});
  vi.stubGlobal('ResizeObserver', class {
    observe() {}
    disconnect() {}
  });
  setLanguage('zh');
  host = document.createElement('div');
  document.body.append(host);
  root = createRoot(host);
  api.profiles.mockResolvedValue([{id: 'source', name: 'Inbox'}, {id: 'shared', name: 'Shared source', sharing: {}}]);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.resetAllMocks();
  vi.unstubAllGlobals();
});

async function click(label: string) {
  const button = [...document.querySelectorAll('button')].find(candidate =>
    candidate.textContent?.trim() === label || candidate.getAttribute('aria-label') === label);
  expect(button, label).toBeTruthy();
  await act(async () => button!.click());
}

it('only assigns owned profiles to joined members and scopes the request to their membership', async () => {
  api.invoke.mockResolvedValue({teams: [owner]});
  await act(async () => root.render(<TeamWorkspace cloud={cloudFor('owner')} sharing={sharing} onProfiles={vi.fn()}/>));
  await click('选择要分配的副本');
  expect(document.querySelector('[role="listbox"]')?.textContent).not.toContain('Shared source');
  await click('Inbox');
  await click('选择成员');
  expect(document.querySelector('[role="listbox"]')?.textContent).not.toContain('jo@example.test');
  await click('Alex · alex@example.test');
  const form = [...host.querySelectorAll('form')].find(item => item.textContent?.includes('分配副本'))!;
  await act(async () => form.dispatchEvent(new Event('submit', {bubbles: true, cancelable: true})));
  expect(sharing.act).toHaveBeenCalledWith(expect.objectContaining({
    profileId: 'source', includesSession: false, note: '', email: 'alex@example.test',
    team: {accountId: 'owner', teamId: 'team', memberId: 'active'},
  }));
});

it('shows only the pending invitation to the invitee and accepts the exact membership', async () => {
  const invite: Team = {...owner, role: 'member', status: 'pending', membershipId: 'invite', members: []};
  let joined = false;
  api.invoke.mockImplementation(async (_command: string, args: {action: TeamAction}) => {
    if (args.action.type === 'accept') {joined = true; return {ok: true};}
    return {teams: [joined ? {...invite, status: 'active'} : invite]};
  });
  await act(async () => root.render(<TeamWorkspace cloud={cloudFor('member')} sharing={sharing} onProfiles={vi.fn()}/>));
  expect([...host.querySelectorAll('button')].some(button => button.textContent === '邀请成员')).toBe(false);
  await click('加入团队');
  expect(api.invoke).toHaveBeenCalledWith('team_action', {
    accountId: 'member', action: {type: 'accept', teamId: 'team', memberId: 'invite'},
  });
  expect(host.textContent).toContain('退出团队');
  expect([...host.querySelectorAll('button')].some(button => button.textContent === '分配副本')).toBe(false);
});

it('does not render an old account roster after switching accounts', async () => {
  let finish!: (result: {teams: Team[]}) => void;
  api.invoke.mockReturnValueOnce(new Promise(resolve => {finish = resolve;})).mockResolvedValueOnce({teams: []});
  await act(async () => root.render(<TeamWorkspace cloud={cloudFor('old')} sharing={sharing} onProfiles={vi.fn()}/>));
  await act(async () => root.render(<TeamWorkspace cloud={cloudFor('new')} sharing={sharing} onProfiles={vi.fn()}/>));
  await act(async () => finish({teams: [owner]}));
  expect(host.textContent).not.toContain('Support');
  expect(host.textContent).toContain('还没有团队');
});
