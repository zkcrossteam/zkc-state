package server

import (
	"context"
	"fmt"

	core "github.com/envoyproxy/go-control-plane/envoy/config/core/v3"
	auth "github.com/envoyproxy/go-control-plane/envoy/service/auth/v3"
	envoy_type "github.com/envoyproxy/go-control-plane/envoy/type/v3"

	"google.golang.org/genproto/googleapis/rpc/status"
	"google.golang.org/grpc/codes"
)

type Server struct{}

func denied(code int32, body string) *auth.CheckResponse {
	return &auth.CheckResponse{
		Status: &status.Status{Code: code},
		HttpResponse: &auth.CheckResponse_DeniedResponse{
			DeniedResponse: &auth.DeniedHttpResponse{
				Status: &envoy_type.HttpStatus{
					Code: envoy_type.StatusCode(code),
				},
				Body: body,
			},
		},
	}
}

func allowed() *auth.CheckResponse {
	return &auth.CheckResponse{
		Status: &status.Status{Code: int32(codes.OK)},
		HttpResponse: &auth.CheckResponse_OkResponse{
			OkResponse: &auth.OkHttpResponse{
				Headers: []*core.HeaderValueOption{
					{
						Header: &core.HeaderValue{
							Key:   "x-auth-contract-id",
							Value: "FX6glXnwnPljB/ayPW/WHDz/EjB21Ewn4um+3wITXoc=",
						},
					},
				},
				HeadersToRemove: []string{"token"},
			},
		},
	}
}

// Check implements Envoy Authorization service. Proto file:
// https://github.com/envoyproxy/envoy/blob/main/api/envoy/service/auth/v3/external_auth.proto
func (a *Server) Check(ctx context.Context, req *auth.CheckRequest) (*auth.CheckResponse, error) {
	headers := req.Attributes.Request.Http.Headers

	fmt.Println("=== Request headers ===")
	for h, v := range headers {
		fmt.Printf("%s: %s\n", h, v)
	}
	fmt.Println("=======================")

	// TODO: Do some check here.
	// if headers["token"] != "abc" {
	// 	return denied(401, "unauthenticated"), nil
	// }

	return allowed(), nil
}
