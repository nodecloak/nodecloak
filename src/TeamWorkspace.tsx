import {useEffect, useRef, useState, type FormEvent, type ReactNode} from 'react';
import {ArrowRight, CircleAlert, Plus, RefreshCw, UsersRound} from 'lucide-react';
import type {Controller} from './AccountWorkspace';
import {native} from './bridge';
import {listProfiles, type BrowserProfile} from './browser-profiles';
import {t} from './i18n';
import {SharingWorkspace, type SharingController} from './ProfileSharing';
import {shareDeadline} from './profile-sharing';
import SelectMenu from './SelectMenu';
import {teamAction, teamError, type Team, type TeamAction, type TeamMember} from './teams';
import './teams.css';

interface Props {
  cloud: Controller;
  sharing: SharingController;
  onProfiles: () => void;
}

export default function TeamWorkspace(props: Props) {
  const accountId = props.cloud.status.account?.id;
  if (!native || !accountId) {
    return <section className="sharing-access">
      <span className="sharing-feature"><UsersRound size={25}/></span>
      <h3>{t('登录后使用团队')}</h3>
      <p>{t('在桌面客户端登录 NodeCloak，即可创建团队或接受邀请。')}</p>
      {native && <button className="button primary" onClick={() => props.cloud.open('account')}>{t('登录 NodeCloak')}</button>}
    </section>;
  }
  return <SignedInTeams key={accountId} {...props} accountId={accountId}/>;
}

function SignedInTeams({accountId, cloud, sharing, onProfiles}: Props & {accountId: string}) {
  const [teams, setTeams] = useState<Team[]>([]);
  const [selectedId, setSelectedId] = useState('');
  const [newName, setNewName] = useState('');
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const revision = useRef(0);
  const pending = useRef(false);

  async function refresh() {
    const request = ++revision.current;
    setLoading(true);
    setError('');
    try {
      const data = await teamAction<{teams: Team[]}>(accountId, {type: 'list'});
      if (request === revision.current) setTeams(data.teams);
    } catch (err) {
      if (request === revision.current) setError(teamError(err));
    } finally {
      if (request === revision.current) setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
    return () => {revision.current++;};
  }, [accountId]);

  async function mutate(action: TeamAction): Promise<boolean> {
    if (pending.current) return false;
    pending.current = true;
    setBusy(true);
    setError('');
    const request = ++revision.current;
    try {
      const result = await teamAction<{id?: string}>(accountId, action);
      if (request !== revision.current) return false;
      if (action.type === 'create' && result.id) setSelectedId(result.id);
      await refresh();
      if (action.type === 'remove') void sharing.refresh();
      return true;
    } catch (err) {
      if (request === revision.current) setError(teamError(err));
      return false;
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }

  async function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (newName.trim() && await mutate({type: 'create', name: newName.trim()})) setNewName('');
  }

  const team = teams.find(item => item.id === selectedId) ?? teams[0];
  const disabled = busy || loading || sharing.busy;
  return <div className="team-workspace" aria-busy={loading || busy}>
    <header className="ap-hero team-hero">
      <div className="ap-hero-icon"><UsersRound size={25}/></div>
      <div className="ap-hero-copy">
        <span className="eyebrow">TEAM WORKSPACES</span>
        <h2>{t('一起管理浏览器副本。')}</h2>
        <p>{t('邀请成员加入团队，为每个人分配需要使用的浏览器副本。')}</p>
      </div>
      <button className="button secondary" disabled={disabled} onClick={() => {void refresh(); void sharing.refresh();}}>
        <RefreshCw size={15} className={loading ? 'spin' : ''}/>{t('刷新')}
      </button>
    </header>
    {error && <div className="notice warning-notice" role="alert"><CircleAlert size={17}/><p>{t(error)}</p></div>}
    <div className="team-layout">
      <section className="sharing-card team-list" aria-label={t('我的团队')}>
        <h3>{t('我的团队')}</h3>
        {teams.map(item => <button type="button" className="team-choice" key={item.id}
          aria-pressed={team?.id === item.id} disabled={busy || sharing.busy}
          onClick={() => {sharing.clearError(); setSelectedId(item.id);}}>
          <strong>{item.name}</strong>
          <small>{t(item.status === 'pending' ? '待接受邀请' : item.role === 'owner' ? '创建者' : '成员')}</small>
        </button>)}
        {teams.length === 0 && <p className="sharing-hint">{t(loading ? '正在读取团队…' : '还没有团队')}</p>}
        <form className="sharing-fields team-form" onSubmit={event => void create(event)}>
          <label>{t('团队名称')}
            <input value={newName} maxLength={60} required disabled={disabled} onChange={e => setNewName(e.target.value)}/>
          </label>
          <button className="button primary" disabled={disabled || !newName.trim()}><Plus size={16}/>{t('创建团队')}</button>
        </form>
      </section>
      {team && <TeamDetails key={team.id} team={team} accountId={accountId} disabled={disabled}
        cloud={cloud} sharing={sharing} mutate={mutate} onProfiles={onProfiles}/>}
    </div>
  </div>;
}

function TeamDetails({team, accountId, disabled, cloud, sharing, mutate, onProfiles}: Props & {
  team: Team; accountId: string; disabled: boolean; mutate: (action: TeamAction) => Promise<boolean>;
}) {
  const [name, setName] = useState('');
  const [email, setEmail] = useState('');
  const [removal, setRemoval] = useState<{id: string; name: string}>();
  const owner = team.role === 'owner';
  const active = team.status === 'active';
  const membershipId = team.membershipId;
  const remove = (memberId: string) => mutate({type: 'remove', teamId: team.id, memberId});
  const scopedSharing = {
    ...sharing,
    data: {
      ...sharing.data,
      sent: sharing.data.sent.filter(share => share.teamId === team.id),
      received: sharing.data.received.filter(share => share.teamId === team.id),
    },
  };

  async function invite(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (await mutate({type: 'invite', teamId: team.id, name: name.trim(), email: email.trim()})) {
      setName('');
      setEmail('');
    }
  }

  return <div className="team-detail">
    <section className="sharing-card">
      <div className="team-section-heading">
        <div><h3>{team.name}</h3><p>{t('创建者')} · {team.ownerName}</p></div>
        {active && !owner && membershipId && <button className="button secondary compact" disabled={disabled}
          onClick={() => setRemoval({id: membershipId, name: team.name})}>{t('退出团队')}</button>}
      </div>
      {!active && membershipId && <div className="team-row">
        <p>{t('接受邀请后，创建者可以为你分配副本。')}</p>
        <button className="button secondary compact" disabled={disabled} onClick={() => void remove(membershipId)}>{t('拒绝')}</button>
        <button className="button primary compact" disabled={disabled}
          onClick={() => void mutate({type: 'accept', teamId: team.id, memberId: membershipId})}>{t('加入团队')}</button>
      </div>}
      {owner && <>
        <div className="team-members">
          <h4>{t('团队成员')}</h4>
          {team.members.length === 0 && <p className="sharing-hint">{t('邀请已注册的 NodeCloak 用户加入团队。')}</p>}
          {team.members.map(member => <div className="team-row" key={member.id}>
            <div><strong>{member.name}</strong><p>{member.email}</p></div>
            <span className={`sharing-status ${member.status === 'active' ? 'accepted' : 'pending'}`}>
              {t(member.status === 'active' ? '已加入' : '待接受邀请')}
            </span>
            <button className="button secondary compact" disabled={disabled}
              onClick={() => setRemoval({id: member.id, name: member.name})}>
              {t(member.status === 'active' ? '移除成员' : '取消邀请')}
            </button>
          </div>)}
        </div>
        <form className="sharing-fields team-form" onSubmit={event => void invite(event)}>
          <fieldset className="sharing-fields sharing-form" disabled={disabled}>
            <label>{t('对方用户名')}<input value={name} required maxLength={60} onChange={e => setName(e.target.value)}/></label>
            <label>{t('对方邮箱')}<input type="email" value={email} required maxLength={254} onChange={e => setEmail(e.target.value)}/></label>
          </fieldset>
          <button className="button primary compact" disabled={disabled || !name.trim() || !email.trim()}>{t('邀请成员')}</button>
        </form>
      </>}
      {removal && <div className="notice warning-notice team-confirm" role="group" aria-label={t('确认撤销团队访问')}>
        <CircleAlert size={17}/><div><strong>{removal.name}</strong><p>{t('确认后，此成员的团队副本授权会被撤销。')}</p></div>
        <button className="button secondary compact" disabled={disabled} onClick={() => setRemoval(undefined)}>{t('取消')}</button>
        <button className="button primary compact" disabled={disabled}
          onClick={() => void remove(removal.id).then(ok => {if (ok) setRemoval(undefined);})}>{t('确认')}</button>
      </div>}
    </section>
    {owner && <AssignProfile team={team} accountId={accountId} disabled={disabled} sharing={sharing}/>}
    {active && <section className="team-detail" aria-label={t('团队副本')}>
      <div className="team-section-heading">
        <h3>{t('团队副本')}</h3>
        <button className="button secondary compact" onClick={onProfiles}>{t('前往浏览器副本')}<ArrowRight size={15}/></button>
      </div>
      <SharingWorkspace cloud={cloud} sharing={scopedSharing} initialTab={owner ? 'sent' : 'received'} team/>
    </section>}
  </div>;
}

function AssignProfile({team, accountId, disabled, sharing}: {
  team: Team; accountId: string; disabled: boolean; sharing: SharingController;
}) {
  const [profiles, setProfiles] = useState<BrowserProfile[]>([]);
  const [profileId, setProfileId] = useState('');
  const [memberId, setMemberId] = useState('');
  const [days, setDays] = useState('30');
  const [error, setError] = useState('');
  const members = team.members.filter(member => member.status === 'active');
  const member: TeamMember | undefined = members.find(item => item.id === memberId);
  const profile = profiles.find(item => item.id === profileId);

  useEffect(() => {
    let active = true;
    void listProfiles().then(items => {
      if (active) setProfiles(items.filter(item => !item.sharing && !item.deletedAt));
    }).catch(err => {if (active) setError(teamError(err));});
    return () => {active = false;};
  }, []);

  async function assign(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!member || !profile || disabled) return;
    const result = await sharing.act({
      type: 'create', profileId: profile.id, name: member.name, email: member.email,
      expiresAt: shareDeadline(days, ''), includesSession: false, note: '',
      team: {accountId, teamId: team.id, memberId: member.id},
    });
    if (result) setProfileId('');
  }

  let controls: ReactNode;
  if (!members.length) {
    controls = <p className="sharing-hint">{t('成员接受团队邀请后即可分配副本。')}</p>;
  } else {
    controls = <form className="sharing-fields team-form" onSubmit={event => void assign(event)}>
      <div className="team-assignment">
        <label>{t('副本')}<SelectMenu label={t('选择要分配的副本')} placeholder={t('选择要分配的副本')}
          value={profileId} onChange={setProfileId} disabled={disabled} searchable
          options={profiles.map(item => ({value: item.id, label: item.name}))}/></label>
        <label>{t('成员')}<SelectMenu label={t('选择成员')} placeholder={t('选择成员')}
          value={memberId} onChange={setMemberId} disabled={disabled}
          options={members.map(item => ({value: item.id, label: `${item.name} · ${item.email}`}))}/></label>
        <label>{t('有效期')}<SelectMenu label={t('有效期')} value={days} onChange={setDays} disabled={disabled}
          options={[{value: '7', label: t('7 天')}, {value: '30', label: t('30 天')}, {value: '90', label: t('90 天')}]}/></label>
      </div>
      <button className="button primary compact" disabled={disabled || !member || !profile}>{t('分配副本')}</button>
    </form>;
  }

  return <section className="sharing-card">
    <h3>{t('分配副本')}</h3>
    <p className="sharing-hint">{t('成员收到配置快照，使用系统代理并自行登录网站。团队副本不会包含原副本的登录数据、代理设置和备注。')}</p>
    {error && <p className="profile-error" role="alert">{t(error)}</p>}
    {controls}
  </section>;
}
