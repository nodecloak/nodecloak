package api

import (
	"context"
	"fmt"
	"testing"
	"time"
)

func TestTeamAssignmentsAndMembershipRevocation(t *testing.T) {
	h, owner, member, recipient := sharingUsers(t)
	outsider := h.login("other@example.com", deviceC)["accessToken"].(string)
	require := func(method, path, token string, body any, want int) map[string]any {
		t.Helper()
		status, out := h.req(method, path, token, body)
		if status != want {
			t.Fatalf("%s %s: got %d %v, want %d", method, path, status, out, want)
		}
		return out
	}
	require("GET", "/v1/teams", "", nil, 401)
	team := require("POST", "/v1/teams", owner, map[string]string{"name": "Support"}, 201)["id"].(string)
	path := "/v1/teams/" + team + "/members"
	invitation := map[string]string{"name": "friend", "email": "friend@example.com"}
	require("POST", path, outsider, invitation, 404)
	memberID := require("POST", path, owner, invitation, 201)["id"].(string)
	require("POST", path, owner, invitation, 409)
	require("POST", path+"/"+memberID+"/accept", outsider, nil, 404)
	require("POST", path+"/"+memberID+"/accept", owner, nil, 404)

	assignment := sharingBody(recipient, time.Now().Add(time.Hour))
	assignment["teamId"] = team
	assignment["teamMemberId"] = memberID
	require("POST", "/v1/shares", owner, assignment, 409)
	pending := require("GET", "/v1/teams", member, nil, 200)["teams"].([]any)[0].(map[string]any)
	if pending["role"] != "member" || pending["status"] != "pending" || len(pending["members"].([]any)) != 0 {
		t.Fatal("pending member received the wrong role or roster", pending)
	}
	if teams := require("GET", "/v1/teams", outsider, nil, 200)["teams"].([]any); len(teams) != 0 {
		t.Fatal("unrelated account can see team", teams)
	}
	require("POST", path+"/"+memberID+"/accept", member, nil, 200)
	require("POST", path+"/"+memberID+"/accept", member, nil, 200)
	assignment["includesSession"] = true
	require("POST", "/v1/shares", owner, assignment, 400)
	assignment["includesSession"] = false
	if share := require("POST", "/v1/shares", owner, assignment, 201)["share"].(map[string]any); share["teamId"] != team {
		t.Fatal("team assignment lost its scope", share)
	}
	assignment["id"] = uuid()
	require("POST", "/v1/shares", owner, assignment, 409)
	require("POST", "/v1/shares/"+shareA+"/accept", member, nil, 200)
	require("GET", "/v1/shares/"+shareA+"/payload", member, nil, 200)
	direct := sharingBody(recipient, time.Now().Add(time.Hour))
	directID := uuid()
	direct["id"] = directID
	require("POST", "/v1/shares", owner, direct, 201)
	require("POST", "/v1/shares/"+directID+"/accept", member, nil, 200)
	require("DELETE", path+"/"+memberID, outsider, nil, 404)
	require("DELETE", path+"/"+memberID, owner, nil, 200)
	require("POST", "/v1/shares/"+shareA+"/authorize", member, nil, 409)
	require("GET", "/v1/shares/"+shareA+"/payload", member, nil, 409)
	require("GET", "/v1/shares/"+directID+"/payload", member, nil, 200)
	require("POST", "/v1/shares", owner, assignment, 409)
	var remaining int
	if err := h.db.QueryRow(context.Background(), "SELECT count(*) FROM profile_share_envelopes WHERE share_id=$1", shareA).Scan(&remaining); err != nil || remaining != 0 {
		t.Fatal("revoked envelope still stored", remaining, err)
	}

	for n := 0; n < 4; n++ {
		newID := require("POST", path, owner, invitation, 201)["id"].(string)
		require("POST", path+"/"+memberID+"/accept", member, nil, 404)
		require("POST", path+"/"+newID+"/accept", member, nil, 200)
		require("POST", "/v1/shares", owner, assignment, 409)
		body := sharingBody(recipient, time.Now().Add(time.Hour))
		body["id"] = uuid()
		body["teamId"] = team
		body["teamMemberId"] = newID
		body["sourceProfileId"] = fmt.Sprintf("race-%d", n)
		result := make(chan int, 1)
		go func() {
			s, _ := h.req("POST", "/v1/shares", owner, body)
			result <- s
		}()
		require("DELETE", path+"/"+newID, member, nil, 200)
		if status := <-result; status != 201 && status != 409 {
			t.Fatalf("assignment during departure: %d", status)
		}
		if err := h.db.QueryRow(context.Background(), "SELECT count(*) FROM profile_shares WHERE team_id=$1 AND status IN ('pending','accepted')", team).Scan(&remaining); err != nil || remaining != 0 {
			t.Fatal("member departure left live grants", remaining, err)
		}
	}
}
