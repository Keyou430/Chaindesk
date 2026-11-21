package engine

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"io"
	"net/http"
	"net/netip"
	"strings"
	"time"
)

type HealthResult struct {
	Healthy         bool   `json:"healthy"`
	IPv4            string `json:"ipv4,omitempty"`
	ObservationCode string `json:"observationCode,omitempty"`
	ErrorCode       string `json:"errorCode,omitempty"`
}

type ipSource struct {
	URL   string
	Trace bool
}

type healthTargets struct {
	HTTPS []string
	IP    []ipSource
}

var defaultHealthTargets = healthTargets{
	HTTPS: []string{"https://www.gstatic.com/generate_204", "https://cp.cloudflare.com/generate_204"},
	IP:    []ipSource{{URL: "https://api.ipify.org"}, {URL: "https://www.cloudflare.com/cdn-cgi/trace", Trace: true}},
}

func inspectChainHealth(ctx context.Context, client *http.Client, targets healthTargets, expected string) HealthResult {
	result := HealthResult{}
	if expected != "" && validObservedIPv4(expected) == "" {
		result.ErrorCode = "EXPECTED_IP_INVALID"
		return result
	}
	for _, target := range targets.HTTPS {
		requestContext, cancel := context.WithTimeout(ctx, 4*time.Second)
		request, err := http.NewRequestWithContext(requestContext, http.MethodGet, target, nil)
		if err != nil {
			cancel()
			result.ErrorCode = "PROBE_TARGET_INVALID"
			return result
		}
		request.Header.Set("Cache-Control", "no-cache")
		response, err := client.Do(request)
		if err == nil {
			response.Body.Close()
			if response.StatusCode == http.StatusNoContent {
				result.Healthy, result.ErrorCode = true, ""
			} else if response.StatusCode >= 300 && response.StatusCode < 400 {
				result.ErrorCode = "PROBE_REDIRECT"
			} else {
				result.ErrorCode = "PROBE_HTTP_STATUS"
			}
		} else {
			result.ErrorCode = probeFailureCode(err)
		}
		cancel()
		if result.Healthy {
			break
		}
		if ctx.Err() != nil {
			result.ErrorCode = "PROBE_CANCELED"
			return result
		}
	}
	if !result.Healthy {
		if result.ErrorCode == "" {
			result.ErrorCode = "PROBE_UNAVAILABLE"
		}
		return result
	}
	observations := make([]string, 0, len(targets.IP))
	for _, source := range targets.IP {
		requestContext, cancel := context.WithTimeout(ctx, 2*time.Second)
		request, err := http.NewRequestWithContext(requestContext, http.MethodGet, source.URL, nil)
		if err == nil {
			request.Header.Set("Cache-Control", "no-cache")
			var response *http.Response
			response, err = client.Do(request)
			if err == nil {
				body, readErr := io.ReadAll(io.LimitReader(response.Body, 4097))
				response.Body.Close()
				if response.StatusCode == http.StatusOK && readErr == nil && len(body) <= 4096 {
					value := strings.TrimSpace(string(body))
					if source.Trace {
						value = ""
						for _, line := range strings.Split(string(body), "\n") {
							if strings.HasPrefix(line, "ip=") {
								value = strings.TrimSpace(strings.TrimPrefix(line, "ip="))
								break
							}
						}
					}
					if address := validObservedIPv4(value); address != "" {
						observations = append(observations, address)
					}
				}
			}
		}
		cancel()
	}
	if ctx.Err() != nil {
		return HealthResult{ErrorCode: "PROBE_CANCELED"}
	}
	if len(observations) != 2 {
		result.ObservationCode = "IP_UNAVAILABLE"
	} else if observations[0] != observations[1] {
		result.ObservationCode = "IP_INCONSISTENT"
	} else {
		result.IPv4 = observations[0]
	}
	if expected != "" {
		if result.ObservationCode != "" {
			result.Healthy, result.ErrorCode = false, result.ObservationCode
		} else if result.IPv4 != expected {
			result.Healthy, result.ErrorCode = false, "IP_MISMATCH"
		}
	}
	return result
}

func validObservedIPv4(value string) string {
	address, err := netip.ParseAddr(value)
	if err != nil || !address.Is4() || address.IsUnspecified() || address.IsMulticast() {
		return ""
	}
	return address.String()
}

func probeFailureCode(err error) string {
	if errors.Is(err, context.Canceled) {
		return "PROBE_CANCELED"
	}
	var tlsError *tls.CertificateVerificationError
	var unknownAuthority x509.UnknownAuthorityError
	if errors.As(err, &tlsError) || errors.As(err, &unknownAuthority) {
		return "PROBE_TLS_FAILED"
	}
	if errors.Is(err, context.DeadlineExceeded) {
		return "PROBE_TIMEOUT"
	}
	return "PROBE_UNAVAILABLE"
}
