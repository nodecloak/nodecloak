package api

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"time"

	"github.com/jackc/pgx/v5"
)

// Snapshots and individually wrapped keys are opaque to the service. No package
// is included in metadata, and only an accepted, addressed approved device can read it.
type shareSummary struct {
	ID              string     `json:"id"`
	TeamID          string     `json:"teamId,omitempty"`
	SourceProfileID string     `json:"sourceProfileId"`
	ProfileName     string     `json:"profileName"`
	Browser         string     `json:"browser"`
	SenderID        string     `json:"senderId"`
	SenderName      string     `json:"senderName"`
	SenderEmail     string     `json:"senderEmail"`
	RecipientID     string     `json:"recipientId"`
	RecipientName   string     `json:"recipientName"`
	RecipientEmail  string     `json:"recipientEmail"`
	IncludesSession bool       `json:"includesSession"`
	Note            string     `json:"note"`
	Status          string     `json:"status"`
	CreatedAt       time.Time  `json:"createdAt"`
	ExpiresAt       time.Time  `json:"expiresAt"`
	AcceptedAt      *time.Time `json:"acceptedAt"`
	DeviceAccess    string     `json:"deviceAccess,omitempty"`
}

const shareColumns = `id::text,source_profile_id,profile_name,browser,sender_id::text,sender_name,sender_email,recipient_id::text,recipient_name,recipient_email,includes_session,note,status,created_at,expires_at,accepted_at,COALESCE(team_id::text,'')`

func scanShare(row pgx.Row, v *shareSummary) error {
	return row.Scan(&v.ID, &v.SourceProfileID, &v.ProfileName, &v.Browser, &v.SenderID, &v.SenderName, &v.SenderEmail, &v.RecipientID, &v.RecipientName, &v.RecipientEmail, &v.IncludesSession, &v.Note, &v.Status, &v.CreatedAt, &v.ExpiresAt, &v.AcceptedAt, &v.TeamID)
}
func (s *Server) registerSharing() {
	s.mux.HandleFunc("POST /v1/shares/recipient", s.auth(false, s.shareRecipient))
	s.mux.HandleFunc("POST /v1/shares", s.auth(false, s.shareCreate))
	s.mux.HandleFunc("GET /v1/shares", s.auth(false, s.shareList))
	s.mux.HandleFunc("PATCH /v1/shares/{id}", s.auth(false, s.shareExtend))
	s.mux.HandleFunc("GET /v1/shares/{id}/payload", s.auth(false, s.sharePayload))
	s.mux.HandleFunc("POST /v1/shares/{id}/authorize", s.auth(false, s.shareAuthorize))
	for _, action := range []string{"accept", "decline", "revoke", "leave"} {
		a := action
		s.mux.HandleFunc("POST /v1/shares/{id}/"+a, s.auth(false, func(w http.ResponseWriter, r *http.Request, i identity) { s.shareTransition(w, r, i, a) }))
	}
}

type shareDevice struct {
	ID        string `json:"id"`
	PublicKey string `json:"publicKey"`
}

func (s *Server) shareRecipient(w http.ResponseWriter, r *http.Request, i identity) {
	var in struct {
		Name  string `json:"name"`
		Email string `json:"email"`
	}
	if !decode(w, r, &in, 4096) {
		return
	}
	email, ok := normalizedEmail(in.Email)
	name := strings.TrimSpace(in.Name)
	if !ok || name == "" || len([]rune(name)) > 60 {
		failure(w, 400, "invalid_recipient")
		return
	}
	if !s.limit(r.Context(), "share-recipient:"+i.Account, 20) {
		failure(w, 429, "try_later")
		return
	}
	var a struct {
		ID    string `json:"id"`
		Name  string `json:"name"`
		Email string `json:"email"`
	}
	e := s.db.QueryRow(r.Context(), "SELECT id::text,display_name,email FROM users WHERE email=$1 AND display_name=$2 AND NOT disabled AND id<>$3", email, name, i.Account).Scan(&a.ID, &a.Name, &a.Email)
	if errors.Is(e, pgx.ErrNoRows) {
		failure(w, 404, "recipient_not_found")
		return
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	rows, e := s.db.Query(r.Context(), "SELECT id::text,public_key FROM devices WHERE account_id=$1 AND status<>'revoked' ORDER BY id LIMIT 101", a.ID)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer rows.Close()
	devices := []shareDevice{}
	for rows.Next() {
		var d shareDevice
		if rows.Scan(&d.ID, &d.PublicKey) != nil {
			failure(w, 503, "database_unavailable")
			return
		}
		devices = append(devices, d)
	}
	if rows.Err() != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if len(devices) == 0 {
		failure(w, 409, "recipient_device_unavailable")
		return
	}
	if len(devices) > 100 {
		failure(w, 409, "too_many_devices")
		return
	}
	reply(w, 200, map[string]any{"account": a, "devices": devices})
}
func validShareDeadline(deadline, now time.Time) bool {
	return deadline.After(now) && !deadline.After(now.Add(366*24*time.Hour))
}
func shareLive(status string) bool { return status == "pending" || status == "accepted" }

// All participating users are locked in ID order before a grant is locked.
// This follows account revocation's account-first order without sender/recipient deadlocks.
func (s *Server) shareLockAccounts(ctx context.Context, tx pgx.Tx, i identity, other string, approved bool) error {
	rows, e := tx.Query(ctx, "SELECT id FROM users WHERE id=$1 OR id=$2 ORDER BY id FOR NO KEY UPDATE", i.Account, other)
	if e != nil {
		return e
	}
	for rows.Next() {
	}
	e = rows.Err()
	rows.Close()
	if e != nil {
		return e
	}
	if s.authorizeTx(ctx, tx, i, approved) != nil {
		return pgx.ErrNoRows
	}
	var enabled bool
	if e = tx.QueryRow(ctx, "SELECT NOT disabled FROM users WHERE id=$1", i.Account).Scan(&enabled); e != nil || !enabled {
		return pgx.ErrNoRows
	}
	return nil
}
func (s *Server) shareCreate(w http.ResponseWriter, r *http.Request, i identity) {
	release, allowed := beginSnapshotTransfer(w, r, snapshotWireLimit)
	if !allowed {
		return
	}
	defer release()
	var in struct {
		ID              string    `json:"id"`
		TeamID          string    `json:"teamId"`
		TeamMemberID    string    `json:"teamMemberId"`
		SourceProfileID string    `json:"sourceProfileId"`
		ProfileName     string    `json:"profileName"`
		Browser         string    `json:"browser"`
		RecipientID     string    `json:"recipientId"`
		RecipientName   string    `json:"recipientName"`
		RecipientEmail  string    `json:"recipientEmail"`
		ExpiresAt       time.Time `json:"expiresAt"`
		IncludesSession bool      `json:"includesSession"`
		Note            string    `json:"note"`
		Payload         string    `json:"payload"`
		Writer          string    `json:"writer"`
		Envelopes       []struct {
			DeviceID  string `json:"deviceId"`
			PublicKey string `json:"publicKey"`
			Envelope  string `json:"envelope"`
		} `json:"envelopes"`
	}
	if !decode(w, r, &in, snapshotWireLimit) {
		return
	}
	now := s.now().UTC()
	email, ok := normalizedEmail(in.RecipientEmail)
	in.ProfileName = strings.TrimSpace(in.ProfileName)
	in.RecipientName = strings.TrimSpace(in.RecipientName)
	if !validID(in.ID) || !validID(in.RecipientID) || in.RecipientID == i.Account || !ok || in.RecipientName == "" || len([]rune(in.RecipientName)) > 60 || !validShareDeadline(in.ExpiresAt, now) || in.Writer != i.Device || len(in.SourceProfileID) == 0 || len(in.SourceProfileID) > 256 || in.ProfileName == "" || len([]rune(in.ProfileName)) > 60 || (in.Browser != "chrome" && in.Browser != "edge" && in.Browser != "firefox") || len(in.Note) > 2000 || len(in.Payload) < 16 || int64(len(in.Payload)) > snapshotWireLimit || len(in.Envelopes) == 0 || len(in.Envelopes) > 100 {
		failure(w, 400, "invalid_share")
		return
	}
	if (in.TeamID == "") != (in.TeamMemberID == "") || in.TeamID != "" && (!validID(in.TeamID) || !validID(in.TeamMemberID) || in.IncludesSession) {
		failure(w, 400, "invalid_team_assignment")
		return
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	if s.shareLockAccounts(r.Context(), tx, i, in.RecipientID, false) != nil {
		failure(w, 401, "unauthorized")
		return
	}
	if in.TeamID != "" {
		// Serialize assignment creation with removal and protect the recipient's
		// membership across encryption uploads made from a stale member list.
		ctx := r.Context()
		owner, err := teamOwner(ctx, tx, in.TeamID)
		if errors.Is(err, pgx.ErrNoRows) || err == nil && owner != i.Account {
			failure(w, 404, "team_not_found")
			return
		}
		if err != nil {
			failure(w, 503, "database_unavailable")
			return
		}
		var active bool
		err = tx.QueryRow(ctx, "SELECT EXISTS(SELECT 1 FROM team_memberships WHERE team_id=$1 AND user_id=$2 AND id=$3 AND status='active')", in.TeamID, in.RecipientID, in.TeamMemberID).Scan(&active)
		if err != nil {
			failure(w, 503, "database_unavailable")
			return
		}
		if !active {
			failure(w, 409, "team_member_inactive")
			return
		}
	}
	// Waiting for a competing account operation must not admit a deadline
	// which elapsed before the invitation is actually persisted.
	now = s.now().UTC()
	if !validShareDeadline(in.ExpiresAt, now) {
		failure(w, 400, "invalid_deadline")
		return
	}
	var senderName, senderEmail string
	if e = tx.QueryRow(r.Context(), "SELECT display_name,email FROM users WHERE id=$1", i.Account).Scan(&senderName, &senderEmail); e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	var recipient string
	e = tx.QueryRow(r.Context(), "SELECT id::text FROM users WHERE id=$1 AND email=$2 AND display_name=$3 AND NOT disabled", in.RecipientID, email, in.RecipientName).Scan(&recipient)
	if errors.Is(e, pgx.ErrNoRows) {
		failure(w, 404, "recipient_not_found")
		return
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	seen := map[string]bool{}
	for _, d := range in.Envelopes {
		if !validID(d.DeviceID) || seen[d.DeviceID] || len(d.Envelope) < 16 || len(d.Envelope) > 16384 {
			failure(w, 400, "invalid_envelope")
			return
		}
		seen[d.DeviceID] = true
		var matched bool
		e = tx.QueryRow(r.Context(), "SELECT EXISTS(SELECT 1 FROM devices WHERE account_id=$1 AND id=$2 AND public_key=$3 AND status<>'revoked')", recipient, d.DeviceID, d.PublicKey).Scan(&matched)
		if e != nil {
			failure(w, 503, "database_unavailable")
			return
		}
		if !matched {
			failure(w, 409, "recipient_device_changed")
			return
		}
	}
	var count int
	var size int64
	if _, e = tx.Exec(r.Context(), "WITH expired AS (UPDATE profile_shares SET status='expired',payload='' WHERE sender_id=$1 AND status IN ('pending','accepted') AND expires_at<=$2 RETURNING id) DELETE FROM profile_share_envelopes WHERE share_id IN (SELECT id FROM expired)", i.Account, now); e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if e = tx.QueryRow(r.Context(), "SELECT count(*),COALESCE(sum(octet_length(payload)),0) FROM profile_shares WHERE sender_id=$1 AND status IN ('pending','accepted')", i.Account).Scan(&count, &size); e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if count >= 1000 || size+int64(len(in.Payload)) > 256<<20 {
		failure(w, 413, "share_quota_exceeded")
		return
	}
	var exists bool
	if tx.QueryRow(r.Context(), "SELECT EXISTS(SELECT 1 FROM profile_shares WHERE id=$1)", in.ID).Scan(&exists) != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if exists {
		failure(w, 409, "share_id_conflict")
		return
	}
	inserted, e := tx.Exec(r.Context(), `INSERT INTO profile_shares(id,sender_id,recipient_id,source_profile_id,profile_name,browser,sender_name,sender_email,recipient_name,recipient_email,includes_session,note,status,created_at,expires_at,payload,writer,team_id)
		VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'pending',$13,$14,$15,$16,NULLIF($17,'')::uuid)
		ON CONFLICT(team_id,recipient_id,source_profile_id) WHERE team_id IS NOT NULL AND status IN ('pending','accepted') DO NOTHING`, in.ID, i.Account, recipient, in.SourceProfileID, in.ProfileName, in.Browser, senderName, senderEmail, in.RecipientName, email, in.IncludesSession, in.Note, now, in.ExpiresAt, in.Payload, in.Writer, in.TeamID)
	if e == nil && inserted.RowsAffected() == 0 {
		failure(w, 409, "team_assignment_exists")
		return
	}
	for _, d := range in.Envelopes {
		if e == nil {
			_, e = tx.Exec(r.Context(), "INSERT INTO profile_share_envelopes(share_id,device_id,public_key,envelope) VALUES($1,$2,$3,$4)", in.ID, d.DeviceID, d.PublicKey, d.Envelope)
		}
	}
	var share shareSummary
	if e == nil {
		e = scanShare(tx.QueryRow(r.Context(), "SELECT "+shareColumns+" FROM profile_shares WHERE id=$1", in.ID), &share)
	}
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	s.audit(r.Context(), i, "share_created", in.ID)
	reply(w, 201, map[string]any{"share": share})
}
func (s *Server) shareList(w http.ResponseWriter, r *http.Request, i identity) {
	now := s.now().UTC()
	// Terminal expiry is persisted so a later deadline edit cannot resurrect a grant.
	_, e := s.db.Exec(r.Context(), "WITH expired AS (UPDATE profile_shares SET status='expired',payload='' WHERE (sender_id=$1 OR recipient_id=$1) AND status IN ('pending','accepted') AND expires_at<=$2 RETURNING id) DELETE FROM profile_share_envelopes WHERE share_id IN (SELECT id FROM expired)", i.Account, now)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	rows, e := s.db.Query(r.Context(), "SELECT "+shareColumns+" FROM profile_shares WHERE sender_id=$1 OR recipient_id=$1 ORDER BY created_at DESC,id", i.Account)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer rows.Close()
	sent := []shareSummary{}
	received := []shareSummary{}
	for rows.Next() {
		var share shareSummary
		if scanShare(rows, &share) != nil {
			failure(w, 503, "database_unavailable")
			return
		}
		if share.SenderID == i.Account {
			sent = append(sent, share)
		} else {
			received = append(received, share)
		}
	}
	if rows.Err() != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	// Pending devices receive only invitation metadata until explicitly authorized
	// for the addressed grant. Personal cloud device approval remains independent.
	deviceRows, e := s.db.Query(r.Context(), `SELECT e.share_id::text,(e.authorized_at IS NOT NULL OR COALESCE(d.status='approved' AND d.epoch=v.epoch,false)) FROM profile_share_envelopes e JOIN profile_shares p ON p.id=e.share_id JOIN devices d ON d.account_id=p.recipient_id AND d.id=e.device_id AND d.public_key=e.public_key AND d.status<>'revoked' LEFT JOIN vaults v ON v.account_id=d.account_id WHERE p.recipient_id=$1 AND e.device_id=$2`, i.Account, i.Device)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	access := map[string]string{}
	for deviceRows.Next() {
		var id string
		var ready bool
		if e = deviceRows.Scan(&id, &ready); e != nil {
			break
		}
		access[id] = "authorizationRequired"
		if ready {
			access[id] = "ready"
		}
	}
	if e == nil {
		e = deviceRows.Err()
	}
	deviceRows.Close()
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	for n := range received {
		received[n].DeviceAccess = access[received[n].ID]
		if received[n].DeviceAccess == "" {
			received[n].DeviceAccess = "unavailable"
		}
	}
	reply(w, 200, map[string]any{"sent": sent, "received": received, "serverTime": now})
}

// shareBegin resolves visibility without exposing existence to unrelated accounts,
// locks accounts and grant, and permanently settles an elapsed deadline.
func (s *Server) shareBegin(w http.ResponseWriter, r *http.Request, i identity, role string, approved bool) (pgx.Tx, shareSummary, bool) {
	var share shareSummary
	id := r.PathValue("id")
	if !validID(id) {
		failure(w, 400, "invalid_share")
		return nil, share, false
	}
	condition := "(sender_id=$2 OR recipient_id=$2)"
	if role == "sender" {
		condition = "sender_id=$2"
	}
	if role == "recipient" {
		condition = "recipient_id=$2"
	}
	e := scanShare(s.db.QueryRow(r.Context(), "SELECT "+shareColumns+" FROM profile_shares WHERE id=$1 AND "+condition, id, i.Account), &share)
	if errors.Is(e, pgx.ErrNoRows) {
		failure(w, 404, "not_found")
		return nil, share, false
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return nil, share, false
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return nil, share, false
	}
	other := share.SenderID
	if other == i.Account {
		other = share.RecipientID
	}
	if s.shareLockAccounts(r.Context(), tx, i, other, approved) != nil {
		tx.Rollback(r.Context())
		failure(w, 401, "unauthorized")
		return nil, share, false
	}
	if e = scanShare(tx.QueryRow(r.Context(), "SELECT "+shareColumns+" FROM profile_shares WHERE id=$1 AND "+condition+" FOR UPDATE", id, i.Account), &share); e != nil {
		tx.Rollback(r.Context())
		failure(w, 404, "not_found")
		return nil, share, false
	}
	if shareLive(share.Status) && !share.ExpiresAt.After(s.now()) {
		_, e = tx.Exec(r.Context(), "UPDATE profile_shares SET status='expired',payload='' WHERE id=$1", id)
		if e == nil {
			_, e = tx.Exec(r.Context(), "DELETE FROM profile_share_envelopes WHERE share_id=$1", id)
		}
		if e == nil {
			e = tx.Commit(r.Context())
		}
		if e != nil {
			tx.Rollback(r.Context())
			failure(w, 503, "database_unavailable")
		} else {
			failure(w, 409, "share_expired")
		}
		return nil, share, false
	}
	return tx, share, true
}
func (s *Server) shareDeviceAccess(ctx context.Context, tx pgx.Tx, i identity, share shareSummary, withPayload bool) (string, string, string, error) {
	if share.Status != "accepted" && share.Status != "pending" {
		return "", "", "", errors.New("share_inactive")
	}
	var payload, envelope, writer string
	e := tx.QueryRow(ctx, `SELECT CASE WHEN $4 THEN p.payload ELSE '' END,e.envelope,p.writer::text FROM profile_shares p JOIN profile_share_envelopes e ON e.share_id=p.id AND e.device_id=$2 JOIN devices d ON d.account_id=p.recipient_id AND d.id=e.device_id AND d.public_key=e.public_key AND d.status<>'revoked' LEFT JOIN vaults v ON v.account_id=d.account_id JOIN users u ON u.id=p.sender_id AND NOT u.disabled WHERE p.id=$1 AND p.recipient_id=$3 AND (e.authorized_at IS NOT NULL OR (d.status='approved' AND d.epoch=v.epoch))`, share.ID, i.Device, i.Account, withPayload).Scan(&payload, &envelope, &writer)
	return payload, envelope, writer, e
}
func (s *Server) shareTransition(w http.ResponseWriter, r *http.Request, i identity, action string) {
	var consent struct {
		AuthorizeDevice bool `json:"authorizeDevice"`
	}
	if action == "accept" && r.ContentLength != 0 && !decode(w, r, &consent, 4096) {
		return
	}
	role := "recipient"
	if action == "revoke" {
		role = "sender"
	}
	tx, share, ok := s.shareBegin(w, r, i, role, false)
	if !ok {
		return
	}
	defer tx.Rollback(r.Context())
	target := map[string]string{"accept": "accepted", "decline": "declined", "revoke": "revoked", "leave": "left"}[action]
	allowed := share.Status == target || action == "accept" && share.Status == "pending" || action == "decline" && share.Status == "pending" || action == "leave" && share.Status == "accepted" || action == "revoke" && shareLive(share.Status)
	if !allowed {
		failure(w, 409, "share_inactive")
		return
	}
	if action == "accept" {
		var ready bool
		e := tx.QueryRow(r.Context(), `SELECT (e.authorized_at IS NOT NULL OR COALESCE(d.status='approved' AND d.epoch=v.epoch,false)) FROM profile_share_envelopes e JOIN devices d ON d.account_id=$3 AND d.id=e.device_id AND d.public_key=e.public_key AND d.status<>'revoked' LEFT JOIN vaults v ON v.account_id=d.account_id JOIN users u ON u.id=$4 AND NOT u.disabled WHERE e.share_id=$1 AND e.device_id=$2`, share.ID, i.Device, i.Account, share.SenderID).Scan(&ready)
		if errors.Is(e, pgx.ErrNoRows) {
			failure(w, 403, "share_device_unavailable")
			return
		}
		if e != nil {
			failure(w, 503, "database_unavailable")
			return
		}
		if !ready && !consent.AuthorizeDevice {
			failure(w, 403, "share_authorization_required")
			return
		}
		if consent.AuthorizeDevice {
			if _, e = tx.Exec(r.Context(), "UPDATE profile_share_envelopes SET authorized_at=COALESCE(authorized_at,$3) WHERE share_id=$1 AND device_id=$2", share.ID, i.Device, s.now().UTC()); e != nil {
				failure(w, 503, "database_unavailable")
				return
			}
		}
		if _, _, _, e := s.shareDeviceAccess(r.Context(), tx, i, share, false); e != nil {
			failure(w, 403, "share_device_unavailable")
			return
		}
	}
	var e error
	if share.Status != target {
		if action == "accept" {
			_, e = tx.Exec(r.Context(), "UPDATE profile_shares SET status='accepted',accepted_at=$2 WHERE id=$1", share.ID, s.now().UTC())
		} else {
			_, e = tx.Exec(r.Context(), "UPDATE profile_shares SET status=$2,payload='' WHERE id=$1", share.ID, target)
			if e == nil {
				_, e = tx.Exec(r.Context(), "DELETE FROM profile_share_envelopes WHERE share_id=$1", share.ID)
			}
		}
	}
	if e == nil {
		e = scanShare(tx.QueryRow(r.Context(), "SELECT "+shareColumns+" FROM profile_shares WHERE id=$1", share.ID), &share)
	}
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	s.audit(r.Context(), i, "share_"+target, share.ID)
	reply(w, 200, map[string]any{"share": share})
}
func (s *Server) shareExtend(w http.ResponseWriter, r *http.Request, i identity) {
	var in struct {
		ExpiresAt time.Time `json:"expiresAt"`
	}
	if !decode(w, r, &in, 4096) {
		return
	}
	if !validShareDeadline(in.ExpiresAt, s.now()) {
		failure(w, 400, "invalid_deadline")
		return
	}
	tx, share, ok := s.shareBegin(w, r, i, "sender", false)
	if !ok {
		return
	}
	defer tx.Rollback(r.Context())
	if !shareLive(share.Status) {
		failure(w, 409, "share_inactive")
		return
	}
	if !validShareDeadline(in.ExpiresAt, s.now()) {
		failure(w, 400, "invalid_deadline")
		return
	}
	_, e := tx.Exec(r.Context(), "UPDATE profile_shares SET expires_at=$2 WHERE id=$1", share.ID, in.ExpiresAt)
	if e == nil {
		e = scanShare(tx.QueryRow(r.Context(), "SELECT "+shareColumns+" FROM profile_shares WHERE id=$1", share.ID), &share)
	}
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	s.audit(r.Context(), i, "share_deadline_updated", share.ID)
	reply(w, 200, map[string]any{"share": share})
}
func (s *Server) shareRead(w http.ResponseWriter, r *http.Request, i identity, withPayload bool) {
	tx, share, ok := s.shareBegin(w, r, i, "recipient", false)
	if !ok {
		return
	}
	defer tx.Rollback(r.Context())
	if share.Status != "accepted" {
		failure(w, 409, "share_inactive")
		return
	}
	payload, envelope, writer, e := s.shareDeviceAccess(r.Context(), tx, i, share, withPayload)
	if e != nil {
		failure(w, 403, "share_device_unavailable")
		return
	}
	if e = tx.Commit(r.Context()); e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	out := map[string]any{"share": share, "serverTime": s.now().UTC()}
	if withPayload {
		out["payload"] = payload
		out["envelope"] = envelope
		out["writer"] = writer
	}
	reply(w, 200, out)
}
func (s *Server) sharePayload(w http.ResponseWriter, r *http.Request, i identity) {
	release, allowed := beginSnapshotTransfer(w, r, snapshotWireLimit)
	if !allowed {
		return
	}
	defer release()
	s.shareRead(w, r, i, true)
}
func (s *Server) shareAuthorize(w http.ResponseWriter, r *http.Request, i identity) {
	s.shareRead(w, r, i, false)
}
