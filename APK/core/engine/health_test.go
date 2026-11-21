package engine

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestChainHealthRejectsPortalRedirectAndIPMismatch(t *testing.T) {
	for _, scenario := range []struct {
		name, firstIP, secondIP, expected, code string
		status                                  int
		healthy                                 bool
	}{
		{"healthy", "203.0.113.8", "ip=203.0.113.8\n", "203.0.113.8", "", 204, true},
		{"portal", "203.0.113.8", "ip=203.0.113.8\n", "", "PROBE_HTTP_STATUS", 200, false},
		{"redirect", "203.0.113.8", "ip=203.0.113.8\n", "", "PROBE_REDIRECT", 302, false},
		{"mismatch", "203.0.113.8", "ip=203.0.113.8\n", "203.0.113.9", "IP_MISMATCH", 204, false},
		{"inconsistent", "203.0.113.8", "ip=203.0.113.9\n", "203.0.113.8", "IP_INCONSISTENT", 204, false},
		{"ipv6", "2001:db8::8", "ip=2001:db8::8\n", "203.0.113.8", "IP_UNAVAILABLE", 204, false},
		{"optional-ip-unavailable", "unavailable", "ip=unavailable\n", "", "", 204, true},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				switch r.URL.Path {
				case "/reach":
					w.Header().Set("Location", "/portal")
					w.WriteHeader(scenario.status)
				case "/first":
					w.Write([]byte(scenario.firstIP))
				case "/second":
					w.Write([]byte(scenario.secondIP))
				}
			}))
			defer server.Close()
			client := server.Client()
			client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
			result := inspectChainHealth(context.Background(), client, healthTargets{
				HTTPS: []string{server.URL + "/reach"},
				IP:    []ipSource{{URL: server.URL + "/first"}, {URL: server.URL + "/second", Trace: true}},
			}, scenario.expected)
			if result.Healthy != scenario.healthy || result.ErrorCode != scenario.code {
				t.Fatalf("unexpected result: %+v", result)
			}
			if scenario.name == "optional-ip-unavailable" && result.ObservationCode != "IP_UNAVAILABLE" {
				t.Fatalf("missing optional observation failure: %+v", result)
			}
		})
	}
}

func TestChainHealthRejectsUntrustedTLSAndCanceledWork(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(204) }))
	defer server.Close()
	targets := healthTargets{HTTPS: []string{server.URL}}
	if result := inspectChainHealth(context.Background(), &http.Client{}, targets, ""); result.Healthy || result.ErrorCode != "PROBE_TLS_FAILED" {
		t.Fatalf("untrusted certificate accepted: %+v", result)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if result := inspectChainHealth(ctx, server.Client(), targets, ""); result.Healthy || result.ErrorCode != "PROBE_CANCELED" {
		t.Fatalf("canceled work accepted: %+v", result)
	}
}
