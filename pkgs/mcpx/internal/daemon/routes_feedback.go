package daemon

import (
	"encoding/json"
	"fmt"
	"math"
	"net/http"
	"strconv"
	"strings"

	"github.com/dezren39/mcpx/internal/logstore"
)

type feedbackSubmitReq struct {
	TraceID        string             `json:"traceId"`
	Score          float64            `json:"score"`
	Target         string             `json:"target,omitempty"`
	RetrievalScore *float64           `json:"retrievalScore,omitempty"`
	ExecutionScore *float64           `json:"executionScore,omitempty"`
	Scores         map[string]float64 `json:"scores,omitempty"`
	Notes          string             `json:"notes,omitempty"`
}

// handleFeedbackSubmit implements POST /v1/feedback.
func (s *Server) handleFeedbackSubmit(w http.ResponseWriter, r *http.Request) {
	fbStore := s.reg.FeedbackStore()
	if fbStore == nil {
		writeErr(w, http.StatusServiceUnavailable, fmt.Errorf("feedback store not initialized"))
		return
	}

	var req feedbackSubmitReq
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		writeErr(w, http.StatusBadRequest, fmt.Errorf("invalid json request: %w", err))
		return
	}
	if req.TraceID == "" {
		writeErr(w, http.StatusBadRequest, fmt.Errorf("traceId is required"))
		return
	}

	params := logstore.FeedbackParams{
		TraceID:        req.TraceID,
		Score:          req.Score,
		Target:         req.Target,
		RetrievalScore: req.RetrievalScore,
		ExecutionScore: req.ExecutionScore,
		Scores:         req.Scores,
		Notes:          req.Notes,
	}

	if err := fbStore.ProvideFeedback(params); err != nil {
		writeErr(w, http.StatusInternalServerError, fmt.Errorf("failed to save feedback: %w", err))
		return
	}

	resp := map[string]any{
		"status":  "ok",
		"traceId": req.TraceID,
		"score":   req.Score,
	}
	if req.Target != "" {
		resp["target"] = req.Target
	}
	if req.RetrievalScore != nil {
		resp["retrievalScore"] = *req.RetrievalScore
	}
	if req.ExecutionScore != nil {
		resp["executionScore"] = *req.ExecutionScore
	}
	writeJSON(w, http.StatusOK, resp)
}

// handleInteractionsList implements GET /v1/interactions.
func (s *Server) handleInteractionsList(w http.ResponseWriter, r *http.Request) {
	fbStore := s.reg.FeedbackStore()
	if fbStore == nil {
		writeJSON(w, http.StatusOK, []any{})
		return
	}

	q := r.URL.Query()
	filter := logstore.InteractionFilter{
		Query:     q.Get("q"),
		Source:    q.Get("source"),
		SessionID: q.Get("sessionId"),
		Target:    q.Get("target"),
		SortBy:    q.Get("sortBy"),
		SortDesc:  q.Get("desc") != "0",
	}

	if hasFbStr := q.Get("hasFeedback"); hasFbStr != "" {
		hasFb := hasFbStr == "1" || strings.ToLower(hasFbStr) == "true"
		filter.HasFeedback = &hasFb
	}
	// Invalid query values are rejected with 400 naming the parameter
	// (#2223): silently dropping a bound made a typo'd minScore return the
	// unfiltered list with 200, and a NaN/Inf bound (which ParseFloat
	// accepts) reached SQLite as NULL and matched nothing — both read as
	// real answers. The same silent-drop shape is fixed for limit/offset.
	parseScoreBound := func(name string) (*float64, bool) {
		raw := q.Get(name)
		if raw == "" {
			return nil, true
		}
		val, err := strconv.ParseFloat(raw, 64)
		if err != nil || math.IsNaN(val) || math.IsInf(val, 0) {
			writeErr(w, http.StatusBadRequest, fmt.Errorf("invalid %s %q: must be a finite number", name, raw))
			return nil, false
		}
		return &val, true
	}
	parseIntParam := func(name string) (int, bool) {
		raw := q.Get(name)
		if raw == "" {
			return 0, true
		}
		n, err := strconv.Atoi(raw)
		if err != nil {
			writeErr(w, http.StatusBadRequest, fmt.Errorf("invalid %s %q: must be an integer", name, raw))
			return 0, false
		}
		return n, true
	}
	var ok bool
	if filter.MinScore, ok = parseScoreBound("minScore"); !ok {
		return
	}
	if filter.MaxScore, ok = parseScoreBound("maxScore"); !ok {
		return
	}
	if filter.Limit, ok = parseIntParam("limit"); !ok {
		return
	}
	if filter.Offset, ok = parseIntParam("offset"); !ok {
		return
	}

	list, err := fbStore.List(filter)
	if err != nil {
		writeErr(w, http.StatusInternalServerError, err)
		return
	}
	if list == nil {
		list = []logstore.InteractionRecord{}
	}
	writeJSON(w, http.StatusOK, list)
}

// handleInteractionGet implements GET /v1/interactions/{id}.
func (s *Server) handleInteractionGet(w http.ResponseWriter, r *http.Request) {
	fbStore := s.reg.FeedbackStore()
	if fbStore == nil {
		writeErr(w, http.StatusServiceUnavailable, fmt.Errorf("feedback store not initialized"))
		return
	}

	id := r.PathValue("id")
	if id == "" {
		id = r.URL.Query().Get("id")
	}
	if id == "" {
		writeErr(w, http.StatusBadRequest, fmt.Errorf("id is required"))
		return
	}

	rec, err := fbStore.Get(id)
	if err != nil {
		writeErr(w, http.StatusNotFound, fmt.Errorf("interaction not found: %w", err))
		return
	}
	writeJSON(w, http.StatusOK, rec)
}
