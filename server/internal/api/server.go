package api

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	_ "embed"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgxpool"
	"image/png"
	"io"
	"math/big"
	"net"
	"net/http"
	"net/mail"
	"net/url"
	"strings"
	"time"
)

//go:embed schema.sql
var schema string

//go:embed admin.sql
var adminSchema string

//go:embed announcements.sql
var announcementsSchema string

//go:embed account_security.sql
var securitySchema string

//go:embed sharing.sql
var sharingSchema string

//go:embed mcp.sql
var mcpSchema string

//go:embed teams.sql
var teamsSchema string

func Migrate(ctx context.Context, db *pgxpool.Pool) error {
	tx, e := db.Begin(ctx)
	if e != nil {
		return e
	}
	defer tx.Rollback(ctx)
	if _, e = tx.Exec(ctx, `SELECT pg_advisory_xact_lock(748913226);CREATE TABLE IF NOT EXISTS schema_versions(version integer PRIMARY KEY,applied_at timestamptz NOT NULL DEFAULT now())`); e != nil {
		return e
	}
	var exists bool
	if e = tx.QueryRow(ctx, `SELECT EXISTS(SELECT 1 FROM schema_versions WHERE version=1)`).Scan(&exists); e != nil {
		return e
	}
	if !exists {
		if _, e = tx.Exec(ctx, schema); e != nil {
			return e
		}
		if _, e = tx.Exec(ctx, `INSERT INTO schema_versions(version) VALUES(1)`); e != nil {
			return e
		}
	}
	var v2 bool
	if e = tx.QueryRow(ctx, `SELECT EXISTS(SELECT 1 FROM schema_versions WHERE version=2)`).Scan(&v2); e != nil {
		return e
	}
	if !v2 {
		if _, e = tx.Exec(ctx, `ALTER TABLE devices ADD COLUMN IF NOT EXISTS proof text NOT NULL DEFAULT '';INSERT INTO schema_versions(version) VALUES(2)`); e != nil {
			return e
		}
	}
	var v3 bool
	if e = tx.QueryRow(ctx, `SELECT EXISTS(SELECT 1 FROM schema_versions WHERE version=3)`).Scan(&v3); e != nil {
		return e
	}
	if !v3 {
		if _, e = tx.Exec(ctx, securitySchema); e != nil {
			return e
		}
		if _, e = tx.Exec(ctx, `INSERT INTO schema_versions(version) VALUES(3)`); e != nil {
			return e
		}
	}

	var v4 bool
	if e = tx.QueryRow(ctx, "SELECT EXISTS(SELECT 1 FROM schema_versions WHERE version=4)").Scan(&v4); e != nil {
		return e
	}
	if !v4 {
		if _, e = tx.Exec(ctx, adminSchema); e != nil {
			return e
		}
		var initial struct {
			Groups []catalogGroup `json:"groups"`
		}
		if e = json.Unmarshal(initialCatalog, &initial); e != nil {
			return e
		}
		if _, e = tx.Exec(ctx, "UPDATE admin_catalog SET groups=$1,published_groups=$1 WHERE singleton=true", initial.Groups); e != nil {
			return e
		}

		if _, e = tx.Exec(ctx, "INSERT INTO schema_versions(version) VALUES(4)"); e != nil {
			return e
		}
	}
	var v5 bool
	if e = tx.QueryRow(ctx, "SELECT EXISTS(SELECT 1 FROM schema_versions WHERE version=5)").Scan(&v5); e != nil {
		return e
	}
	if !v5 {
		if _, e = tx.Exec(ctx, announcementsSchema); e != nil {
			return e
		}
		if _, e = tx.Exec(ctx, "INSERT INTO schema_versions(version) VALUES(5)"); e != nil {
			return e
		}
	}
	var v6 bool
	if e = tx.QueryRow(ctx, "SELECT EXISTS(SELECT 1 FROM schema_versions WHERE version=6)").Scan(&v6); e != nil {
		return e
	}
	if !v6 {
		if _, e = tx.Exec(ctx, sharingSchema); e != nil {
			return e
		}
		if _, e = tx.Exec(ctx, "INSERT INTO schema_versions(version) VALUES(6)"); e != nil {
			return e
		}
	}
	var v7 bool
	if e = tx.QueryRow(ctx, "SELECT EXISTS(SELECT 1 FROM schema_versions WHERE version=7)").Scan(&v7); e != nil {
		return e
	}
	if !v7 {
		if _, e = tx.Exec(ctx, "ALTER TABLE profile_share_envelopes ADD COLUMN authorized_at timestamptz;INSERT INTO schema_versions(version) VALUES(7)"); e != nil {
			return e
		}
	}
	var v8 bool
	if e = tx.QueryRow(ctx, "SELECT EXISTS(SELECT 1 FROM schema_versions WHERE version=8)").Scan(&v8); e != nil {
		return e
	}
	if !v8 {
		if _, e = tx.Exec(ctx, mcpSchema); e != nil {
			return e
		}
		if _, e = tx.Exec(ctx, "INSERT INTO schema_versions(version) VALUES(8)"); e != nil {
			return e
		}
	}
	var v9 bool
	if e = tx.QueryRow(ctx, "SELECT EXISTS(SELECT 1 FROM schema_versions WHERE version=9)").Scan(&v9); e != nil {
		return e
	}
	if !v9 {
		if _, e = tx.Exec(ctx, teamsSchema); e != nil {
			return e
		}
		if _, e = tx.Exec(ctx, "INSERT INTO schema_versions(version) VALUES(9)"); e != nil {
			return e
		}
	}
	return tx.Commit(ctx)
}

type Mailer interface {
	Send(context.Context, string, string) error
}
type Config struct {
	PublicURL, TokenPepper, GoogleClientID, GoogleClientSecret string
	TrustedProxies                                             []string
}
type Server struct {
	db   *pgxpool.Pool
	cfg  Config
	mail Mailer
	mux  *http.ServeMux
	http *http.Client
	now  func() time.Time
}

func New(db *pgxpool.Pool, c Config, m Mailer) *Server {
	s := &Server{db: db, cfg: c, mail: m, http: &http.Client{Timeout: 20 * time.Second}, now: time.Now}
	s.mux = http.NewServeMux()
	s.mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, r *http.Request) {
		if s.db.Ping(r.Context()) != nil {
			failure(w, 503, "database_unavailable")
			return
		}
		reply(w, 200, map[string]any{"status": "ok"})
	})
	s.mux.HandleFunc("POST /v1/auth/email/request", s.emailRequest)
	s.mux.HandleFunc("POST /v1/auth/email/verify", s.emailVerify)
	s.mux.HandleFunc("POST /v1/auth/password", s.passwordLogin)
	s.mux.HandleFunc("POST /v1/auth/2fa", s.twoFactorLogin)
	s.mux.HandleFunc("POST /v1/account/password", s.auth(false, s.setPassword))
	s.mux.HandleFunc("POST /v1/account/google/unlink", s.auth(false, s.unlinkGoogle))
	s.mux.HandleFunc("POST /v1/account/2fa/start", s.auth(false, s.startTwoFactor))
	s.mux.HandleFunc("POST /v1/account/2fa/enable", s.auth(false, s.enableTwoFactor))
	s.mux.HandleFunc("POST /v1/account/2fa/disable", s.auth(false, s.disableTwoFactor))
	s.mux.HandleFunc("POST /v1/auth/google/start", s.googleStart)
	s.mux.HandleFunc("GET /v1/auth/google/callback", s.googleCallback)
	s.mux.HandleFunc("POST /v1/auth/google/poll", s.googlePoll)
	s.mux.HandleFunc("POST /v1/auth/refresh", s.refresh)
	s.mux.HandleFunc("GET /v1/account", s.auth(false, s.account))
	s.mux.HandleFunc("PATCH /v1/account", s.auth(false, s.editAccount))
	s.mux.HandleFunc("POST /v1/auth/logout", s.auth(false, s.logout))
	s.mux.HandleFunc("GET /v1/vault", s.auth(false, s.vault))
	s.mux.HandleFunc("POST /v1/vault/initialize", s.auth(false, s.initialize))
	s.mux.HandleFunc("POST /v1/vault/recover", s.auth(false, s.recoverDevice))
	s.mux.HandleFunc("POST /v1/vault/recover/start", s.auth(false, s.recoveryChallenge))
	s.mux.HandleFunc("POST /v1/vault/rotate", s.auth(true, s.rotate))
	s.mux.HandleFunc("GET /v1/devices", s.auth(false, s.devices))
	s.mux.HandleFunc("POST /v1/devices/{id}/approve", s.auth(true, s.approve))
	s.mux.HandleFunc("POST /v1/devices/{id}/revoke", s.auth(true, s.revoke))
	s.mux.HandleFunc("GET /v1/objects", s.auth(true, s.manifest))
	s.mux.HandleFunc("GET /v1/objects/{id}", s.auth(true, s.getObject))
	s.mux.HandleFunc("PUT /v1/objects/{id}", s.auth(true, s.putObject))
	s.mux.HandleFunc("POST /v1/objects/{id}/lease", s.auth(true, s.lease))
	s.mux.HandleFunc("DELETE /v1/objects/{id}/lease", s.auth(true, s.releaseLease))
	s.registerAdmin()
	s.registerSharing()
	s.registerTeams()
	s.registerMCP()
	return s
}
func (s *Server) Handler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("Referrer-Policy", "no-referrer")
		w.Header().Set("Content-Security-Policy", "default-src 'none'; frame-ancestors 'none'")
		publicRead := (r.Method == "GET" || r.Method == "HEAD") && (r.URL.Path == "/v1/catalog" || strings.HasPrefix(r.URL.Path, "/v1/catalog/icons/"))
		if publicRead {
			w.Header().Set("Access-Control-Allow-Origin", "*")
		}
		adminRequest := strings.HasPrefix(r.URL.Path, "/v1/admin/") || ((r.Method == "GET" || r.Method == "HEAD") && strings.HasPrefix(r.URL.Path, "/admin/"))
		mcpBrowser := s.mcpBrowserOrigin(r)
		if r.Header.Get("Origin") != "" && !publicRead && !mcpBrowser && (!adminRequest || !s.adminOrigin(r)) {
			failure(w, 403, "browser_origin_denied")
			return
		}
		s.mux.ServeHTTP(w, r)
	})
}
func reply(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}
func failure(w http.ResponseWriter, status int, code string) {
	reply(w, status, map[string]string{"error": code})
}
func decode(w http.ResponseWriter, r *http.Request, v any, limit int64) bool {
	r.Body = http.MaxBytesReader(w, r.Body, limit)
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	e := dec.Decode(v)
	if e == nil {
		var extra any
		if dec.Decode(&extra) != io.EOF {
			e = errors.New("extra input")
		}
	}
	if e != nil {
		var large *http.MaxBytesError
		if errors.As(e, &large) {
			failure(w, 413, "request_too_large")
		} else {
			failure(w, 400, "invalid_request")
		}
		return false
	}
	return true
}
func token() string {
	b := make([]byte, 32)
	if _, e := rand.Read(b); e != nil {
		panic(e)
	}
	return base64.RawURLEncoding.EncodeToString(b)
}
func uuid() string {
	b := make([]byte, 16)
	if _, e := rand.Read(b); e != nil {
		panic(e)
	}
	b[6] = b[6]&15 | 64
	b[8] = b[8]&63 | 128
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[:4], b[4:6], b[6:8], b[8:10], b[10:])
}
func validID(v string) bool {
	if len(v) != 36 {
		return false
	}
	_, e := hex.DecodeString(strings.ReplaceAll(v, "-", ""))
	return e == nil && v[8] == '-' && v[13] == '-' && v[18] == '-' && v[23] == '-'
}
func (s *Server) hash(v string) string {
	h := hmac.New(sha256.New, []byte(s.cfg.TokenPepper))
	_, _ = h.Write([]byte(v))
	return hex.EncodeToString(h.Sum(nil))
}
func normalizedEmail(v string) (string, bool) {
	v = strings.ToLower(strings.TrimSpace(v))
	a, e := mail.ParseAddress(v)
	return v, e == nil && a.Address == v && len(v) <= 254
}
func (s *Server) limit(ctx context.Context, key string, max int) bool {
	var n int
	e := s.db.QueryRow(ctx, `INSERT INTO auth_limits(key,count) VALUES($1,1) ON CONFLICT(key) DO UPDATE SET count=CASE WHEN auth_limits.started_at<now()-interval '1 minute' THEN 1 ELSE auth_limits.count+1 END,started_at=CASE WHEN auth_limits.started_at<now()-interval '1 minute' THEN now() ELSE auth_limits.started_at END RETURNING count`, s.hash(key)).Scan(&n)
	return e == nil && n <= max
}

type deviceInput struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	PublicKey string `json:"publicKey"`
	Secret    string `json:"secret"`
}

func (d deviceInput) valid() bool {
	pub, e := base64.StdEncoding.DecodeString(d.PublicKey)
	secret, se := base64.StdEncoding.DecodeString(d.Secret)
	return validID(d.ID) && len(strings.TrimSpace(d.Name)) > 0 && len(d.Name) <= 100 && e == nil && len(pub) == 32 && se == nil && len(secret) == 32
}
func (s *Server) emailRequest(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Email string `json:"email"`
	}
	if !decode(w, r, &in, 4096) {
		return
	}
	email, ok := normalizedEmail(in.Email)
	if !ok {
		failure(w, 400, "invalid_email")
		return
	}
	if !s.limit(r.Context(), "email:"+email, 2) || !s.limit(r.Context(), "ip:"+s.clientIP(r), 10) {
		failure(w, 429, "try_later")
		return
	}
	if s.mail == nil {
		failure(w, 503, "email_not_configured")
		return
	}
	n, e := rand.Int(rand.Reader, big.NewInt(1000000))
	if e != nil {
		failure(w, 503, "entropy_unavailable")
		return
	}
	code := fmt.Sprintf("%06d", n.Int64())
	_, e = s.db.Exec(r.Context(), `INSERT INTO email_codes(email,code_hash,expires_at) VALUES($1,$2,now()+interval '10 minutes') ON CONFLICT(email) DO UPDATE SET code_hash=$2,expires_at=now()+interval '10 minutes',attempts=0,issued_at=now()`, email, s.hash("otp:"+email+":"+code))
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if e = s.mail.Send(r.Context(), email, code); e != nil {
		_, _ = s.db.Exec(r.Context(), "DELETE FROM email_codes WHERE email=$1 AND code_hash=$2", email, s.hash("otp:"+email+":"+code))
		failure(w, 503, "email_delivery_failed")
		return
	}
	reply(w, 202, map[string]bool{"sent": true})
}
func (s *Server) emailVerify(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Email  string      `json:"email"`
		Code   string      `json:"code"`
		Device deviceInput `json:"device"`
	}
	if !decode(w, r, &in, 8192) {
		return
	}
	email, ok := normalizedEmail(in.Email)
	if !ok || len(in.Code) != 6 {
		failure(w, 401, "invalid_code")
		return
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	var hash string
	var attempts int
	var live bool
	e = tx.QueryRow(r.Context(), "SELECT code_hash,attempts,expires_at>now() FROM email_codes WHERE email=$1 FOR UPDATE", email).Scan(&hash, &attempts, &live)
	if e != nil || attempts >= 5 || !live {
		failure(w, 401, "invalid_code")
		return
	}
	if subtle.ConstantTimeCompare([]byte(hash), []byte(s.hash("otp:"+email+":"+in.Code))) != 1 {
		_, _ = tx.Exec(r.Context(), "UPDATE email_codes SET attempts=attempts+1 WHERE email=$1", email)
		_ = tx.Commit(r.Context())
		failure(w, 401, "invalid_code")
		return
	}
	if !in.Device.valid() {
		failure(w, 400, "invalid_device")
		return
	}
	_, e = tx.Exec(r.Context(), "DELETE FROM email_codes WHERE email=$1", email)
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	s.completeLogin(w, r, email, "", in.Device)
}

type account struct {
	ID               string `json:"id"`
	Email            string `json:"email"`
	Name             string `json:"name"`
	Avatar           string `json:"avatar"`
	GoogleLinked     bool   `json:"googleLinked"`
	HasPassword      bool   `json:"hasPassword"`
	TwoFactorEnabled bool   `json:"twoFactorEnabled"`
	RecentAuth       bool   `json:"recentAuth"`
}

func accountResponse(r *http.Request, a account) any {
	// Older clients deserialize account metadata strictly. New security fields
	// are opt-in until those desktop clients are upgraded.
	if r.Header.Get("X-NodeCloak-Account-Security") != "1" {
		return map[string]string{"id": a.ID, "email": a.Email, "name": a.Name, "avatar": a.Avatar}
	}
	return a
}

func (s *Server) completeLogin(w http.ResponseWriter, r *http.Request, email, sub string, d deviceInput) {
	s.completeLoginWithHash(w, r, email, sub, d, s.hash("device:"+d.Secret))
}
func (s *Server) completeLoginWithHash(w http.ResponseWriter, r *http.Request, email, sub string, d deviceInput, secretHash string) {
	method := "email"
	if sub != "" {
		method = "google"
	}
	s.completeVerifiedLogin(w, r, email, sub, d, secretHash, method, "", -1)
}
func (s *Server) completeVerifiedLogin(w http.ResponseWriter, r *http.Request, email, sub string, d deviceInput, secretHash, method, passwordProof string, factorVersion int) {
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	var a account
	var googleSub string
	var avatar []byte
	var passwordHash, totp string
	var version int
	// Email links only after verification by the provider. Unique email/sub constraints prevent competing links.
	var registration, userExisting bool
	if tx.QueryRow(r.Context(), "SELECT registration_enabled FROM admin_settings WHERE singleton=true FOR SHARE").Scan(&registration) != nil || tx.QueryRow(r.Context(), "SELECT EXISTS(SELECT 1 FROM users WHERE email=$1)", email).Scan(&userExisting) != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if !userExisting && !registration {
		failure(w, 403, "registration_disabled")
		return
	}
	var disabled bool
	e = tx.QueryRow(r.Context(), `INSERT INTO users(id,email,display_name,google_sub) VALUES($1,$2,$3,NULLIF($4,'')) ON CONFLICT(email) DO UPDATE SET email=users.email RETURNING id::text,email,display_name,avatar,COALESCE(google_sub,''),password_hash,totp_secret,security_version,disabled`, uuid(), email, strings.Split(email, "@")[0], sub).Scan(&a.ID, &a.Email, &a.Name, &avatar, &googleSub, &passwordHash, &totp, &version, &disabled)
	if e != nil || sub != "" && googleSub != sub {
		failure(w, 409, "account_link_conflict")
		return
	}
	if disabled {
		failure(w, 401, "invalid_credentials")
		return
	}
	if (method == "password" && factorVersion < 0 && (passwordProof == "" || subtle.ConstantTimeCompare([]byte(passwordProof), []byte(passwordHash)) != 1)) || (factorVersion >= 0 && factorVersion != version) {
		failure(w, 401, "invalid_credentials")
		return
	}
	if method != "google" && totp != "" && factorVersion < 0 {
		challenge, err := s.createLoginChallenge(r.Context(), tx, a.ID, method, version, d, secretHash)
		if err == nil {
			err = tx.Commit(r.Context())
		}
		if err != nil {
			failure(w, 429, "try_later")
			return
		}
		reply(w, 202, map[string]any{"twoFactorRequired": true, "challenge": challenge})
		return
	}
	a.GoogleLinked, a.HasPassword, a.TwoFactorEnabled, a.RecentAuth = googleSub != "", passwordHash != "", totp != "", true
	if len(avatar) > 0 {
		a.Avatar = "data:image/png;base64," + base64.StdEncoding.EncodeToString(avatar)
	}
	var devices int
	var existing bool
	if tx.QueryRow(r.Context(), "SELECT count(*) FROM devices WHERE account_id=$1 AND status<>'revoked'", a.ID).Scan(&devices) != nil || tx.QueryRow(r.Context(), "SELECT EXISTS(SELECT 1 FROM devices WHERE account_id=$1 AND id=$2)", a.ID, d.ID).Scan(&existing) != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if !existing && devices >= 100 {
		failure(w, 409, "device_limit")
		return
	}
	keyHash := sha256.Sum256([]byte("nodecloak-device-v1:" + a.ID + ":" + d.ID + ":" + d.PublicKey))
	fp := hex.EncodeToString(keyHash[:])
	_, e = tx.Exec(r.Context(), `INSERT INTO devices(account_id,id,name,public_key,secret_hash,fingerprint,status) VALUES($1,$2,$3,$4,$5,$6,'pending') ON CONFLICT(account_id,id) DO NOTHING`, a.ID, d.ID, d.Name, d.PublicKey, secretHash, fp)
	var storedPub, storedSecret, status string
	if e == nil {
		e = tx.QueryRow(r.Context(), "SELECT public_key,secret_hash,status FROM devices WHERE account_id=$1 AND id=$2 FOR UPDATE", a.ID, d.ID).Scan(&storedPub, &storedSecret, &status)
	}
	if e != nil || status == "revoked" || storedPub != d.PublicKey || subtle.ConstantTimeCompare([]byte(storedSecret), []byte(secretHash)) != 1 {
		failure(w, 403, "device_identity_mismatch")
		return
	}
	access, refresh := token(), token()
	e = s.insertSession(r.Context(), tx, a.ID, d.ID, uuid(), access, refresh)
	if e == nil {
		_, e = tx.Exec(r.Context(), "UPDATE sessions SET auth_method=$2 WHERE access_hash=$1", s.hash(access), method)
	}
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 200, map[string]any{"account": accountResponse(r, a), "deviceId": d.ID, "accessToken": access, "refreshToken": refresh, "expiresIn": 900})
}
func (s *Server) insertSession(ctx context.Context, tx pgx.Tx, a, d, f, access, refresh string) error {
	_, e := tx.Exec(ctx, `INSERT INTO sessions(id,account_id,device_id,family,access_hash,refresh_hash,access_expires,refresh_expires) VALUES($1,$2,$3,$4,$5,$6,now()+interval '15 minutes',now()+interval '30 days')`, uuid(), a, d, f, s.hash(access), s.hash(refresh))
	return e
}

type identity struct{ Account, Device, Session, Status string }
type authed func(http.ResponseWriter, *http.Request, identity)

func (s *Server) auth(approved bool, h authed) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		access := strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")
		if len(access) != 43 {
			failure(w, 401, "unauthorized")
			return
		}
		var i identity
		e := s.db.QueryRow(r.Context(), `SELECT s.account_id::text,s.device_id::text,s.id::text,d.status FROM sessions s JOIN devices d ON d.account_id=s.account_id AND d.id=s.device_id JOIN users u ON u.id=s.account_id WHERE NOT u.disabled AND s.access_hash=$1 AND NOT s.revoked AND s.access_expires>now() AND d.status<>'revoked'`, s.hash(access)).Scan(&i.Account, &i.Device, &i.Session, &i.Status)
		if e != nil {
			failure(w, 401, "unauthorized")
			return
		}
		if approved && i.Status != "approved" {
			failure(w, 403, "device_approval_required")
			return
		}
		h(w, r, i)
	}
}
func (s *Server) refresh(w http.ResponseWriter, r *http.Request) {
	var in struct {
		Refresh string `json:"refreshToken"`
	}
	if !decode(w, r, &in, 4096) {
		return
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	var id, a, d, f, status string
	// Use the same account-first lock order as security changes and device revocation.
	// Re-read the session after obtaining this lock, including replay/revocation checks.
	var accountID string
	if tx.QueryRow(r.Context(), "SELECT account_id::text FROM sessions WHERE refresh_hash=$1", s.hash(in.Refresh)).Scan(&accountID) != nil || tx.QueryRow(r.Context(), "SELECT id::text FROM users WHERE id=$1 AND NOT disabled FOR UPDATE", accountID).Scan(&accountID) != nil {
		failure(w, 401, "unauthorized")
		return
	}
	var revoked bool
	var live bool
	var authAt time.Time
	var method string
	e = tx.QueryRow(r.Context(), `SELECT s.id::text,s.account_id::text,s.device_id::text,s.family::text,s.revoked,s.refresh_expires>now(),d.status,s.auth_at,s.auth_method FROM sessions s JOIN devices d ON d.account_id=s.account_id AND d.id=s.device_id WHERE refresh_hash=$1 FOR UPDATE OF s`, s.hash(in.Refresh)).Scan(&id, &a, &d, &f, &revoked, &live, &status, &authAt, &method)
	if e != nil {
		failure(w, 401, "unauthorized")
		return
	}
	if revoked || !live || status == "revoked" {
		_, _ = tx.Exec(r.Context(), "UPDATE sessions SET revoked=true WHERE family=$1", f)
		_ = tx.Commit(r.Context())
		failure(w, 401, "unauthorized")
		return
	}
	_, e = tx.Exec(r.Context(), "UPDATE sessions SET revoked=true WHERE id=$1", id)
	access, refresh := token(), token()
	if e == nil {
		e = s.insertSession(r.Context(), tx, a, d, f, access, refresh)
	}
	if e == nil {
		_, e = tx.Exec(r.Context(), "UPDATE sessions SET auth_at=$2,auth_method=$3 WHERE access_hash=$1", s.hash(access), authAt, method)
	}
	if e == nil {
		_, e = tx.Exec(r.Context(), "UPDATE totp_setups SET session_id=(SELECT id FROM sessions WHERE access_hash=$2) WHERE session_id=$1", id, s.hash(access))
	}
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 200, map[string]any{"accessToken": access, "refreshToken": refresh, "expiresIn": 900})
}
func (s *Server) account(w http.ResponseWriter, r *http.Request, i identity) {
	var a account
	var avatar []byte
	e := s.db.QueryRow(r.Context(), "SELECT u.id::text,u.email,u.display_name,u.avatar,u.google_sub IS NOT NULL,u.password_hash<>'',u.totp_secret<>'',s.auth_at>now()-interval '15 minutes' AND s.auth_method IN ('google','email','password') FROM users u JOIN sessions s ON s.account_id=u.id WHERE u.id=$1 AND s.id=$2", i.Account, i.Session).Scan(&a.ID, &a.Email, &a.Name, &avatar, &a.GoogleLinked, &a.HasPassword, &a.TwoFactorEnabled, &a.RecentAuth)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if len(avatar) > 0 {
		a.Avatar = "data:image/png;base64," + base64.StdEncoding.EncodeToString(avatar)
	}
	reply(w, 200, accountResponse(r, a))
}
func (s *Server) editAccount(w http.ResponseWriter, r *http.Request, i identity) {
	var in struct {
		Name   string `json:"name"`
		Avatar string `json:"avatar"`
	}
	if !decode(w, r, &in, 360<<10) {
		return
	}
	in.Name = strings.TrimSpace(in.Name)
	if len(in.Name) == 0 || len([]rune(in.Name)) > 60 {
		failure(w, 400, "invalid_name")
		return
	}
	var avatar []byte
	if in.Avatar != "" {
		if !strings.HasPrefix(in.Avatar, "data:image/png;base64,") {
			failure(w, 400, "invalid_avatar")
			return
		}
		var e error
		avatar, e = base64.StdEncoding.DecodeString(strings.TrimPrefix(in.Avatar, "data:image/png;base64,"))
		if e != nil || len(avatar) > 256<<10 {
			failure(w, 400, "invalid_avatar")
			return
		}
		cfg, e := png.DecodeConfig(bytes.NewReader(avatar))
		if e != nil || cfg.Width > 512 || cfg.Height > 512 || cfg.Width < 1 || cfg.Height < 1 {
			failure(w, 400, "invalid_avatar")
			return
		}
		if _, e = png.Decode(bytes.NewReader(avatar)); e != nil {
			failure(w, 400, "invalid_avatar")
			return
		}
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	if s.authorizeTx(r.Context(), tx, i, false) != nil {
		failure(w, 401, "unauthorized")
		return
	}
	_, e = tx.Exec(r.Context(), "UPDATE users SET display_name=$2,avatar=$3 WHERE id=$1", i.Account, in.Name, avatar)
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	s.account(w, r, i)
}
func (s *Server) logout(w http.ResponseWriter, r *http.Request, i identity) {
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	// This request was authenticated before waiting. A refresh may rotate that
	// exact session meanwhile; logout must still revoke every session on this device.
	if _, e = tx.Exec(r.Context(), "SELECT id FROM users WHERE id=$1 FOR UPDATE", i.Account); e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	_, e = tx.Exec(r.Context(), "UPDATE sessions SET revoked=true WHERE account_id=$1 AND device_id=$2", i.Account, i.Device)
	if e == nil {
		e = revokeDeviceMCP(r.Context(), tx, i.Account, i.Device)
	}
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 200, map[string]bool{"ok": true})
}
func (s *Server) vault(w http.ResponseWriter, r *http.Request, i identity) {
	var epoch int
	var recovery, envelope, proof string
	e := s.db.QueryRow(r.Context(), `SELECT v.epoch,v.recovery_envelope,d.envelope,d.proof FROM vaults v JOIN devices d ON d.account_id=v.account_id AND d.id=$2 WHERE v.account_id=$1`, i.Account, i.Device).Scan(&epoch, &recovery, &envelope, &proof)
	if errors.Is(e, pgx.ErrNoRows) {
		reply(w, 200, map[string]any{"initialized": false})
		return
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 200, map[string]any{"initialized": true, "epoch": epoch, "recoveryEnvelope": recovery, "deviceEnvelope": envelope, "deviceProof": proof, "approved": i.Status == "approved"})
}

type enrollment struct {
	Epoch    int    `json:"epoch"`
	Recovery string `json:"recoveryEnvelope"`
	Envelope string `json:"deviceEnvelope"`
	Verifier string `json:"recoveryVerifier"`
	Proof    string `json:"deviceProof"`
}

func (s *Server) initialize(w http.ResponseWriter, r *http.Request, i identity) {
	var in enrollment
	if !decode(w, r, &in, 32768) {
		return
	}
	verifier, ve := base64.StdEncoding.DecodeString(in.Verifier)
	if !validProof(in.Proof) || in.Epoch != 1 || len(in.Recovery) < 16 || len(in.Envelope) < 16 || ve != nil || len(verifier) != ed25519.PublicKeySize {
		failure(w, 400, "invalid_envelope")
		return
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	if s.authorizeTx(r.Context(), tx, i, false) != nil {
		failure(w, 401, "unauthorized")
		return
	}
	// Lock the account to serialize first device races.
	_, e = tx.Exec(r.Context(), "SELECT id FROM users WHERE id=$1 FOR UPDATE", i.Account)
	if e == nil {
		_, e = tx.Exec(r.Context(), "INSERT INTO vaults(account_id,epoch,recovery_envelope,recovery_verifier) VALUES($1,1,$2,$3)", i.Account, in.Recovery, in.Verifier)
	}
	if e != nil {
		failure(w, 409, "vault_already_initialized")
		return
	}
	_, e = tx.Exec(r.Context(), "UPDATE devices SET status='approved',epoch=1,envelope=$3,proof=$4 WHERE account_id=$1 AND id=$2 AND status='pending'", i.Account, i.Device, in.Envelope, in.Proof)
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	s.audit(r.Context(), i, "vault_initialized", "")
	reply(w, 201, map[string]bool{"ok": true})
}

func recoveryMessage(account, device string, epoch int, challenge string) string {
	return fmt.Sprintf("nodecloak-recover-v1:%s:%s:%d:%s", account, device, epoch, challenge)
}
func (s *Server) recoveryChallenge(w http.ResponseWriter, r *http.Request, i identity) {
	if !s.limit(r.Context(), "recover:"+i.Account, 5) {
		failure(w, 429, "try_later")
		return
	}
	var epoch int
	if s.db.QueryRow(r.Context(), "SELECT epoch FROM vaults WHERE account_id=$1", i.Account).Scan(&epoch) != nil {
		failure(w, 409, "vault_not_initialized")
		return
	}
	challenge := token()
	_, e := s.db.Exec(r.Context(), `INSERT INTO recovery_challenges(account_id,device_id,challenge_hash,expires_at) VALUES($1,$2,$3,now()+interval '5 minutes') ON CONFLICT(account_id,device_id) DO UPDATE SET challenge_hash=$3,expires_at=now()+interval '5 minutes'`, i.Account, i.Device, s.hash(challenge))
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 200, map[string]any{"challenge": challenge, "message": recoveryMessage(i.Account, i.Device, epoch, challenge), "epoch": epoch})
}
func (s *Server) recoverDevice(w http.ResponseWriter, r *http.Request, i identity) {
	var in struct {
		Epoch     int    `json:"epoch"`
		Envelope  string `json:"envelope"`
		Challenge string `json:"challenge"`
		Signature string `json:"signature"`
		Proof     string `json:"deviceProof"`
	}
	if !decode(w, r, &in, 16384) {
		return
	}
	if !validProof(in.Proof) || len(in.Envelope) < 16 {
		failure(w, 400, "invalid_envelope")
		return
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	if s.authorizeTx(r.Context(), tx, i, false) != nil {
		failure(w, 401, "unauthorized")
		return
	}
	var verifier string
	var live bool
	var challengeHash string
	e = tx.QueryRow(r.Context(), `SELECT v.recovery_verifier,c.challenge_hash,c.expires_at>now() FROM vaults v JOIN recovery_challenges c ON c.account_id=v.account_id AND c.device_id=$2 WHERE v.account_id=$1 AND v.epoch=$3 FOR UPDATE OF c`, i.Account, i.Device, in.Epoch).Scan(&verifier, &challengeHash, &live)
	pub, pe := base64.StdEncoding.DecodeString(verifier)
	sig, se := base64.StdEncoding.DecodeString(in.Signature)
	if e != nil || pe != nil || se != nil || len(pub) != ed25519.PublicKeySize || !live || subtle.ConstantTimeCompare([]byte(challengeHash), []byte(s.hash(in.Challenge))) != 1 || !ed25519.Verify(pub, []byte(recoveryMessage(i.Account, i.Device, in.Epoch, in.Challenge)), sig) {
		failure(w, 401, "invalid_recovery_proof")
		return
	}
	tag, e := tx.Exec(r.Context(), `UPDATE devices SET status='approved',epoch=$3,envelope=$4,proof=$5 WHERE account_id=$1 AND id=$2 AND status IN ('pending','approved') AND EXISTS(SELECT 1 FROM vaults WHERE account_id=$1 AND epoch=$3)`, i.Account, i.Device, in.Epoch, in.Envelope, in.Proof)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if tag.RowsAffected() != 1 {
		failure(w, 409, "vault_changed")
		return
	}
	_, e = tx.Exec(r.Context(), "DELETE FROM recovery_challenges WHERE account_id=$1 AND device_id=$2", i.Account, i.Device)
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	s.audit(r.Context(), i, "device_recovered", i.Device)
	reply(w, 200, map[string]bool{"ok": true})
}

type device struct {
	ID          string    `json:"id"`
	Name        string    `json:"name"`
	PublicKey   string    `json:"publicKey"`
	Fingerprint string    `json:"fingerprint"`
	Status      string    `json:"status"`
	CreatedAt   time.Time `json:"createdAt"`
	Proof       string    `json:"deviceProof"`
}

func (s *Server) devices(w http.ResponseWriter, r *http.Request, i identity) {
	rows, e := s.db.Query(r.Context(), "SELECT id::text,name,public_key,fingerprint,status,created_at,proof FROM devices WHERE account_id=$1 ORDER BY created_at", i.Account)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer rows.Close()
	list := []device{}
	for rows.Next() {
		var d device
		if rows.Scan(&d.ID, &d.Name, &d.PublicKey, &d.Fingerprint, &d.Status, &d.CreatedAt, &d.Proof) != nil {
			failure(w, 503, "database_unavailable")
			return
		}
		list = append(list, d)
	}
	if rows.Err() != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 200, map[string]any{"devices": list, "currentDevice": i.Device})
}
func (s *Server) approve(w http.ResponseWriter, r *http.Request, i identity) {
	var in struct {
		Fingerprint string `json:"fingerprint"`
		Envelope    string `json:"envelope"`
		Epoch       int    `json:"epoch"`
		Proof       string `json:"deviceProof"`
	}
	if !decode(w, r, &in, 16384) {
		return
	}
	if !validProof(in.Proof) || len(in.Envelope) < 10 || !validID(r.PathValue("id")) {
		failure(w, 400, "invalid_envelope")
		return
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	if s.authorizeTx(r.Context(), tx, i, true) != nil {
		failure(w, 401, "unauthorized")
		return
	}
	tag, e := tx.Exec(r.Context(), `UPDATE devices SET status='approved',envelope=$4,epoch=$5,proof=$6 WHERE account_id=$1 AND id=$2 AND fingerprint=$3 AND status='pending' AND EXISTS(SELECT 1 FROM vaults WHERE account_id=$1 AND epoch=$5)`, i.Account, r.PathValue("id"), in.Fingerprint, in.Envelope, in.Epoch, in.Proof)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if tag.RowsAffected() != 1 {
		failure(w, 409, "device_changed")
		return
	}
	if tx.Commit(r.Context()) != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	s.audit(r.Context(), i, "device_approved", r.PathValue("id"))
	reply(w, 200, map[string]bool{"ok": true})
}
func (s *Server) revoke(w http.ResponseWriter, r *http.Request, i identity) {
	id := r.PathValue("id")
	if !validID(id) {
		failure(w, 400, "invalid_device")
		return
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	if s.authorizeTx(r.Context(), tx, i, true) != nil {
		failure(w, 401, "unauthorized")
		return
	}
	var status string
	if tx.QueryRow(r.Context(), "SELECT status FROM devices WHERE account_id=$1 AND id=$2", i.Account, id).Scan(&status) != nil {
		failure(w, 404, "not_found")
		return
	}
	if status == "approved" {
		failure(w, 409, "rotation_required")
		return
	}
	tag, e := tx.Exec(r.Context(), "UPDATE devices SET status='revoked',envelope='' WHERE account_id=$1 AND id=$2", i.Account, id)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if tag.RowsAffected() == 0 {
		failure(w, 404, "not_found")
		return
	}
	_, e = tx.Exec(r.Context(), "UPDATE sessions SET revoked=true WHERE account_id=$1 AND device_id=$2", i.Account, id)
	if e == nil {
		e = revokeDeviceMCP(r.Context(), tx, i.Account, id)
	}
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	s.audit(r.Context(), i, "device_revoked", id)
	reply(w, 200, map[string]bool{"ok": true})
}
func (s *Server) audit(ctx context.Context, i identity, event, id string) {
	_, _ = s.db.Exec(ctx, "INSERT INTO audit(account_id,device_id,event,object_id) VALUES($1,$2,$3,NULLIF($4,'')::uuid)", i.Account, i.Device, event, id)
}

type object struct {
	ID         string    `json:"id"`
	Kind       string    `json:"kind"`
	Revision   int64     `json:"revision"`
	Epoch      int       `json:"epoch"`
	Deleted    bool      `json:"deleted"`
	Payload    string    `json:"payload,omitempty"`
	Writer     string    `json:"writer"`
	Generation int64     `json:"generation"`
	UpdatedAt  time.Time `json:"updatedAt"`
}

func (s *Server) manifest(w http.ResponseWriter, r *http.Request, i identity) {
	rows, e := s.db.Query(r.Context(), "SELECT id::text,kind,revision,epoch,deleted,writer::text,generation,updated_at FROM objects WHERE account_id=$1 ORDER BY id,kind LIMIT 1001", i.Account)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer rows.Close()
	list := []object{}
	for rows.Next() {
		var o object
		if rows.Scan(&o.ID, &o.Kind, &o.Revision, &o.Epoch, &o.Deleted, &o.Writer, &o.Generation, &o.UpdatedAt) != nil {
			failure(w, 503, "database_unavailable")
			return
		}
		list = append(list, o)
	}
	if rows.Err() != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if len(list) > 1000 {
		failure(w, 413, "too_many_objects")
		return
	}
	reply(w, 200, map[string]any{"objects": list})
}
func (s *Server) getObject(w http.ResponseWriter, r *http.Request, i identity) {
	id := r.PathValue("id")
	kind := r.URL.Query().Get("kind")
	if kind == "" {
		kind = "config"
	}
	if !validID(id) || kind != "config" && kind != "session" {
		failure(w, 400, "invalid_object")
		return
	}
	if kind == "session" {
		release, ok := beginSnapshotTransfer(w, r, snapshotWireLimit)
		if !ok {
			return
		}
		defer release()
	}
	var o object
	e := s.db.QueryRow(r.Context(), "SELECT id::text,kind,revision,epoch,deleted,payload,writer::text,generation,updated_at FROM objects WHERE account_id=$1 AND id=$2 AND kind=$3", i.Account, id, kind).Scan(&o.ID, &o.Kind, &o.Revision, &o.Epoch, &o.Deleted, &o.Payload, &o.Writer, &o.Generation, &o.UpdatedAt)
	if errors.Is(e, pgx.ErrNoRows) {
		failure(w, 404, "not_found")
		return
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 200, o)
}
func (s *Server) putObject(w http.ResponseWriter, r *http.Request, i identity) {
	release, ok := beginSnapshotTransfer(w, r, snapshotWireLimit)
	if !ok {
		return
	}
	defer release()
	var in struct {
		Kind           string `json:"kind"`
		Base           int64  `json:"baseRevision"`
		Epoch          int    `json:"epoch"`
		Deleted        bool   `json:"deleted"`
		Restore        bool   `json:"restore"`
		Payload        string `json:"payload"`
		Generation     int64  `json:"generation"`
		ConfigRevision *int64 `json:"configRevision"`
	}
	if !decode(w, r, &in, snapshotWireLimit) {
		return
	}
	id := r.PathValue("id")
	if !validID(id) || in.Kind != "config" && in.Kind != "session" || in.Base < 0 || in.Epoch < 1 || len(in.Payload) < 1 || in.Generation < 0 || in.Kind == "config" && in.Generation != 0 {
		failure(w, 400, "invalid_object")
		return
	}
	if in.Kind == "config" && int64(len(in.Payload)) > smallSnapshotWireLimit {
		failure(w, 413, "request_too_large")
		return
	}
	if in.ConfigRevision != nil && (in.Kind != "session" || *in.ConfigRevision < 1) {
		failure(w, 400, "invalid_object")
		return
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	if s.authorizeTx(r.Context(), tx, i, true) != nil {
		failure(w, 401, "unauthorized")
		return
	}
	var epoch int
	e = tx.QueryRow(r.Context(), "SELECT epoch FROM vaults WHERE account_id=$1 FOR UPDATE", i.Account).Scan(&epoch)
	if e != nil || epoch != in.Epoch {
		failure(w, 409, "vault_changed")
		return
	}
	var count int
	var total int64
	e = tx.QueryRow(r.Context(), "SELECT count(*),COALESCE(sum(octet_length(payload)),0) FROM objects WHERE account_id=$1", i.Account).Scan(&count, &total)
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	var rev int64
	var deleted bool
	var oldSize int64
	e = tx.QueryRow(r.Context(), "SELECT revision,deleted,octet_length(payload) FROM objects WHERE account_id=$1 AND id=$2 AND kind=$3 FOR UPDATE", i.Account, id, in.Kind).Scan(&rev, &deleted, &oldSize)
	if e != nil && !errors.Is(e, pgx.ErrNoRows) {
		failure(w, 503, "database_unavailable")
		return
	}
	if rev != in.Base || deleted && !in.Deleted && !in.Restore {
		reply(w, 409, map[string]any{"error": "revision_conflict", "revision": rev, "deleted": deleted})
		return
	}
	if count >= 1000 && rev == 0 || total-oldSize+int64(len(in.Payload)) > 256<<20 {
		failure(w, 413, "quota_exceeded")
		return
	}
	if in.Kind == "session" {
		// All object writers hold the vault row lock until commit. Configuration
		// cannot change between this check and the session write.
		var configRevision int64
		var configDeleted bool
		e = tx.QueryRow(r.Context(), "SELECT revision,deleted FROM objects WHERE account_id=$1 AND id=$2 AND kind='config'", i.Account, id).Scan(&configRevision, &configDeleted)
		if e != nil && !errors.Is(e, pgx.ErrNoRows) {
			failure(w, 503, "database_unavailable")
			return
		}
		if e == nil && configDeleted != in.Deleted || in.ConfigRevision != nil && (e != nil || configRevision != *in.ConfigRevision) {
			failure(w, 409, "config_conflict")
			return
		}
		var owner string
		var gen int64
		var live bool
		e = tx.QueryRow(r.Context(), "SELECT owner::text,generation,expires_at>now() FROM leases WHERE account_id=$1 AND object_id=$2 FOR UPDATE", i.Account, id).Scan(&owner, &gen, &live)
		if e != nil || owner != i.Device || gen != in.Generation || !live {
			failure(w, 409, "lease_conflict")
			return
		}
	}
	_, e = tx.Exec(r.Context(), `INSERT INTO objects(account_id,id,kind,revision,epoch,deleted,payload,writer,generation) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(account_id,id,kind) DO UPDATE SET revision=$4,epoch=$5,deleted=$6,payload=$7,writer=$8,generation=$9,updated_at=now()`, i.Account, id, in.Kind, rev+1, in.Epoch, in.Deleted, in.Payload, i.Device, in.Generation)
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	reply(w, 200, map[string]int64{"revision": rev + 1})
}
func (s *Server) lease(w http.ResponseWriter, r *http.Request, i identity) {
	var in struct {
		Takeover   bool `json:"takeover"`
		NewSession bool `json:"newSession"`
	}
	if !decode(w, r, &in, 4096) {
		return
	}
	id := r.PathValue("id")
	if !validID(id) {
		failure(w, 400, "invalid_object")
		return
	}
	tx, e := s.db.Begin(r.Context())
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	defer tx.Rollback(r.Context())
	if s.authorizeTx(r.Context(), tx, i, true) != nil {
		failure(w, 401, "unauthorized")
		return
	}
	_, e = tx.Exec(r.Context(), "SELECT id FROM users WHERE id=$1 FOR UPDATE", i.Account)
	var owner string
	var generation int64
	var expires time.Time
	var live bool
	if e == nil {
		e = tx.QueryRow(r.Context(), "SELECT owner::text,generation,expires_at,expires_at>now() FROM leases WHERE account_id=$1 AND object_id=$2 FOR UPDATE", i.Account, id).Scan(&owner, &generation, &expires, &live)
	}
	if e != nil && !errors.Is(e, pgx.ErrNoRows) {
		failure(w, 503, "database_unavailable")
		return
	}
	if live && owner != i.Device && !in.Takeover {
		reply(w, 409, map[string]any{"error": "lease_held", "owner": owner, "expiresAt": expires})
		return
	}
	if !live || owner != i.Device || in.NewSession {
		generation++
	}
	e = tx.QueryRow(r.Context(), `INSERT INTO leases(account_id,object_id,owner,generation,expires_at) VALUES($1,$2,$3,$4,now()+interval '5 minutes') ON CONFLICT(account_id,object_id) DO UPDATE SET owner=$3,generation=$4,expires_at=now()+interval '5 minutes' RETURNING expires_at`, i.Account, id, i.Device, generation).Scan(&expires)
	if e == nil {
		e = tx.Commit(r.Context())
	}
	if e != nil {
		failure(w, 503, "database_unavailable")
		return
	}
	if in.Takeover {
		s.audit(r.Context(), i, "lease_taken_over", id)
	}
	reply(w, 200, map[string]any{"generation": generation, "expiresAt": expires})
}
func (c Config) Validate() error {
	u, e := url.Parse(c.PublicURL)
	if e != nil || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || u.Path != "" && u.Path != "/" || u.Scheme != "https" && (u.Scheme != "http" || u.Hostname() != "127.0.0.1" && u.Hostname() != "localhost") {
		return errors.New("PUBLIC_URL must use HTTPS (loopback HTTP allowed for development)")
	}
	pepper, e := hex.DecodeString(c.TokenPepper)
	if e != nil || len(pepper) < 32 {
		return errors.New("TOKEN_PEPPER must contain at least 32 random bytes encoded as hex")
	}
	for _, cidr := range c.TrustedProxies {
		if _, _, e := net.ParseCIDR(cidr); e != nil {
			return errors.New("TRUSTED_PROXIES must contain CIDR networks")
		}
	}
	return nil
}

// Clean expired ephemeral authentication rows; retained refresh hashes detect replay for 30 days.
func (s *Server) Cleanup(ctx context.Context) error {
	_, e := s.db.Exec(ctx, `DELETE FROM admin_sessions WHERE expires_at<now();DELETE FROM admin_email_codes WHERE expires_at<now();DELETE FROM login_challenges WHERE expires_at<now();DELETE FROM totp_setups WHERE expires_at<now();DELETE FROM recovery_challenges WHERE expires_at<now();DELETE FROM email_codes WHERE expires_at<now();DELETE FROM oauth_flows WHERE expires_at<now();DELETE FROM auth_limits WHERE started_at<now()-interval '1 day';DELETE FROM sessions WHERE refresh_expires<now()-interval '1 day';DELETE FROM audit WHERE created_at<now()-interval '90 days';UPDATE profile_shares SET status='expired',payload='' WHERE status IN ('pending','accepted') AND expires_at<=now();DELETE FROM profile_share_envelopes e USING profile_shares s WHERE e.share_id=s.id AND s.status NOT IN ('pending','accepted')`)
	if e != nil {
		return e
	}
	_, e = s.db.Exec(ctx, `UPDATE mcp_jobs SET state='failed',error=CASE WHEN state='queued' THEN 'REQUEST_EXPIRED' ELSE 'EXECUTION_INDETERMINATE' END,arguments_sealed='',result_sealed='' WHERE expires_at<=now() AND state IN('queued','dispatched','running','awaiting_approval');UPDATE mcp_jobs j SET arguments_sealed='',result_sealed='' FROM mcp_routes r WHERE j.route_id=r.id AND (j.expires_at<=now() OR r.revoked OR r.expires_at<=now());UPDATE mcp_authorizations SET code_sealed=NULL WHERE expires_at<=now();DELETE FROM mcp_jobs WHERE expires_at<now()-interval '1 day';DELETE FROM mcp_authorizations WHERE expires_at<now()-interval '1 day';DELETE FROM mcp_sessions WHERE expires_at<now()-interval '1 day';DELETE FROM mcp_tokens WHERE refresh_expires<now()-interval '1 day'`)
	return e
}

func (s *Server) clientIP(r *http.Request) string {
	ip, _, e := net.SplitHostPort(r.RemoteAddr)
	if e != nil {
		return r.RemoteAddr
	}
	trusted := func(v string) bool {
		parsed := net.ParseIP(v)
		if parsed == nil {
			return false
		}
		for _, cidr := range s.cfg.TrustedProxies {
			_, network, e := net.ParseCIDR(cidr)
			if e == nil && network.Contains(parsed) {
				return true
			}
		}
		return false
	}
	if !trusted(ip) {
		return ip
	}
	chain := strings.Split(r.Header.Get("X-Forwarded-For"), ",")
	for n := len(chain) - 1; n >= 0; n-- {
		candidate := strings.TrimSpace(chain[n])
		if net.ParseIP(candidate) == nil {
			return ip
		}
		ip = candidate
		if !trusted(ip) {
			return ip
		}
	}
	return ip
}

func validProof(s string) bool {
	b, e := base64.StdEncoding.DecodeString(s)
	return e == nil && len(b) == 32
}
