package api

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"unicode/utf8"

	"github.com/jackc/pgx/v5"
)

type teamMember struct {
	ID     string `json:"id"`
	Name   string `json:"name"`
	Email  string `json:"email"`
	Status string `json:"status"`
}

type teamView struct {
	ID           string       `json:"id"`
	Name         string       `json:"name"`
	OwnerName    string       `json:"ownerName"`
	Role         string       `json:"role"`
	Status       string       `json:"status"`
	MembershipID string       `json:"membershipId,omitempty"`
	Members      []teamMember `json:"members"`
}

func (s *Server) registerTeams() {
	s.mux.HandleFunc("GET /v1/teams", s.auth(false, s.teamList))
	s.mux.HandleFunc("POST /v1/teams", s.auth(false, s.teamCreate))
	s.mux.HandleFunc("POST /v1/teams/{team}/members", s.auth(false, s.teamInvite))
	s.mux.HandleFunc("POST /v1/teams/{team}/members/{member}/accept", s.auth(false, s.teamAccept))
	s.mux.HandleFunc("DELETE /v1/teams/{team}/members/{member}", s.auth(false, s.teamRemove))
}

func (s *Server) teamList(w http.ResponseWriter, r *http.Request, i identity) {
	rows, err := s.db.Query(r.Context(), `
		SELECT t.id::text,t.name,owner.display_name,
		       CASE WHEN t.owner_id=$1 THEN 'owner' ELSE 'member' END,
		       COALESCE(mine.status,'active'),COALESCE(mine.id::text,''),
		       COALESCE(m.id::text,''),COALESCE(member.display_name,''),
		       COALESCE(member.email,''),COALESCE(m.status,'')
		FROM teams t
		JOIN users owner ON owner.id=t.owner_id AND NOT owner.disabled
		LEFT JOIN team_memberships mine ON mine.team_id=t.id AND mine.user_id=$1
		LEFT JOIN team_memberships m ON m.team_id=t.id AND t.owner_id=$1
		LEFT JOIN users member ON member.id=m.user_id
		WHERE t.owner_id=$1 OR mine.id IS NOT NULL
		ORDER BY t.created_at,t.id,member.display_name,member.email`, i.Account)
	if err != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer rows.Close()
	teams := []teamView{}
	for rows.Next() {
		var team teamView
		var member teamMember
		if err := rows.Scan(&team.ID, &team.Name, &team.OwnerName, &team.Role, &team.Status,
			&team.MembershipID, &member.ID, &member.Name, &member.Email, &member.Status); err != nil {
			failure(w, 503, "database_unavailable")
			return
		}
		if len(teams) == 0 || teams[len(teams)-1].ID != team.ID {
			team.Members = []teamMember{}
			teams = append(teams, team)
		}
		if member.ID != "" {
			last := &teams[len(teams)-1]
			last.Members = append(last.Members, member)
		}
	}
	if rows.Err() != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 200, map[string]any{"teams": teams})
}

// Membership and assignment mutations acquire account, then team, then grant locks.
func (s *Server) teamTransaction(w http.ResponseWriter, r *http.Request, i identity) (pgx.Tx, bool) {
	ctx := r.Context()
	tx, err := s.db.Begin(ctx)
	if err != nil {
		failure(w, 503, "database_unavailable")
		return nil, false
	}
	var id string
	err = tx.QueryRow(ctx, "SELECT id::text FROM users WHERE id=$1 AND NOT disabled FOR NO KEY UPDATE", i.Account).Scan(&id)
	if err == nil {
		err = s.authorizeTx(ctx, tx, i, false)
	}
	if err != nil {
		_ = tx.Rollback(ctx)
		if errors.Is(err, pgx.ErrNoRows) {
			failure(w, 401, "unauthorized")
		} else {
			failure(w, 503, "database_unavailable")
		}
		return nil, false
	}
	return tx, true
}

func teamOwner(ctx context.Context, tx pgx.Tx, id string) (string, error) {
	var owner string
	err := tx.QueryRow(ctx, "SELECT owner_id::text FROM teams WHERE id=$1 FOR UPDATE", id).Scan(&owner)
	return owner, err
}

func lockedTeam(w http.ResponseWriter, r *http.Request, tx pgx.Tx) (string, bool) {
	id := r.PathValue("team")
	if !validID(id) {
		failure(w, 404, "team_not_found")
		return "", false
	}
	owner, err := teamOwner(r.Context(), tx, id)
	if errors.Is(err, pgx.ErrNoRows) {
		failure(w, 404, "team_not_found")
		return "", false
	}
	if err != nil {
		failure(w, 503, "database_unavailable")
		return "", false
	}
	return owner, true
}

func (s *Server) teamCreate(w http.ResponseWriter, r *http.Request, i identity) {
	var in struct {
		Name string `json:"name"`
	}
	if !decode(w, r, &in, 4096) {
		return
	}
	name := strings.TrimSpace(in.Name)
	if name == "" || utf8.RuneCountInString(name) > 60 || strings.ContainsRune(name, 0) {
		failure(w, 400, "invalid_team_name")
		return
	}
	tx, ok := s.teamTransaction(w, r, i)
	if !ok {
		return
	}
	ctx := r.Context()
	defer tx.Rollback(ctx)
	var count int
	if err := tx.QueryRow(ctx, "SELECT count(*) FROM teams WHERE owner_id=$1", i.Account).Scan(&count); err != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if count >= 20 {
		failure(w, 409, "team_limit")
		return
	}
	id := uuid()
	_, err := tx.Exec(ctx, "INSERT INTO teams(id,owner_id,name) VALUES($1,$2,$3)", id, i.Account, name)
	if err == nil {
		err = tx.Commit(ctx)
	}
	if err != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 201, map[string]string{"id": id})
}

func (s *Server) teamInvite(w http.ResponseWriter, r *http.Request, i identity) {
	var in struct {
		Name  string `json:"name"`
		Email string `json:"email"`
	}
	if !decode(w, r, &in, 4096) {
		return
	}
	email, ok := normalizedEmail(in.Email)
	name := strings.TrimSpace(in.Name)
	if !ok || name == "" || utf8.RuneCountInString(name) > 60 || strings.ContainsRune(name, 0) {
		failure(w, 400, "invalid_recipient")
		return
	}
	if !s.limit(r.Context(), "team-invite:"+i.Account, 30) {
		failure(w, 429, "try_later")
		return
	}
	tx, ok := s.teamTransaction(w, r, i)
	if !ok {
		return
	}
	ctx := r.Context()
	defer tx.Rollback(ctx)
	owner, ok := lockedTeam(w, r, tx)
	if !ok {
		return
	}
	if owner != i.Account {
		failure(w, 404, "team_not_found")
		return
	}
	var user string
	err := tx.QueryRow(ctx, "SELECT id::text FROM users WHERE email=$1 AND display_name=$2 AND id<>$3 AND NOT disabled", email, name, owner).Scan(&user)
	if errors.Is(err, pgx.ErrNoRows) {
		failure(w, 404, "recipient_not_found")
		return
	}
	if err != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	team := r.PathValue("team")
	var count int
	if err = tx.QueryRow(ctx, "SELECT count(*) FROM team_memberships WHERE team_id=$1", team).Scan(&count); err != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if count >= 50 {
		failure(w, 409, "team_member_limit")
		return
	}
	id := uuid()
	err = tx.QueryRow(ctx, `INSERT INTO team_memberships(id,team_id,user_id,status)
		VALUES($1,$2,$3,'pending') ON CONFLICT(team_id,user_id) DO NOTHING RETURNING id::text`, id, team, user).Scan(&id)
	if errors.Is(err, pgx.ErrNoRows) {
		failure(w, 409, "team_member_exists")
		return
	}
	if err == nil {
		err = tx.Commit(ctx)
	}
	if err != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 201, map[string]string{"id": id})
}

func (s *Server) teamAccept(w http.ResponseWriter, r *http.Request, i identity) {
	s.teamMembershipChange(w, r, i, true)
}

func (s *Server) teamRemove(w http.ResponseWriter, r *http.Request, i identity) {
	s.teamMembershipChange(w, r, i, false)
}

func (s *Server) teamMembershipChange(w http.ResponseWriter, r *http.Request, i identity, accept bool) {
	id := r.PathValue("member")
	if !validID(id) {
		failure(w, 404, "team_member_not_found")
		return
	}
	tx, ok := s.teamTransaction(w, r, i)
	if !ok {
		return
	}
	ctx := r.Context()
	defer tx.Rollback(ctx)
	owner, ok := lockedTeam(w, r, tx)
	if !ok {
		return
	}
	team := r.PathValue("team")
	var user string
	var err error
	if accept {
		err = tx.QueryRow(ctx, `UPDATE team_memberships SET status='active'
			WHERE team_id=$1 AND id=$2 AND user_id=$3
			AND EXISTS(SELECT 1 FROM users WHERE id=$4 AND NOT disabled)
			RETURNING user_id::text`, team, id, i.Account, owner).Scan(&user)
	} else {
		err = tx.QueryRow(ctx, `DELETE FROM team_memberships
			WHERE team_id=$1 AND id=$2 AND (user_id=$3 OR $3=$4)
			RETURNING user_id::text`, team, id, i.Account, owner).Scan(&user)
		if err == nil {
			_, err = tx.Exec(ctx, `WITH revoked AS (
				UPDATE profile_shares SET status='revoked',payload=''
				WHERE team_id=$1 AND recipient_id=$2 AND status IN ('pending','accepted')
				RETURNING id
			) DELETE FROM profile_share_envelopes WHERE share_id IN (SELECT id FROM revoked)`, team, user)
		}
	}
	if errors.Is(err, pgx.ErrNoRows) {
		failure(w, 404, "team_member_not_found")
		return
	}
	if err == nil {
		err = tx.Commit(ctx)
	}
	if err != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 200, map[string]bool{"ok": true})
}
