// SPDX-License-Identifier: Apache-2.0

package endpoint

import (
	"fmt"
	"net"
	"os"
	"strings"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func Parse(ep string) (proto, addr string, err error) {
	if strings.HasPrefix(strings.ToLower(ep), "unix://") || strings.HasPrefix(strings.ToLower(ep), "tcp://") {
		s := strings.SplitN(ep, "://", 2)
		if s[1] != "" {
			return s[0], s[1], nil
		}
		return "", "", status.Error(codes.InvalidArgument, fmt.Sprintf("invalid endpoint: %v", ep))
	}
	return "unix", ep, nil
}

func Listen(endpoint string) (net.Listener, func(), error) {
	proto, addr, err := Parse(endpoint)
	if err != nil {
		return nil, nil, err
	}

	cleanup := func() {}
	if proto == "unix" {
		addr = "/" + addr
		if err := os.Remove(addr); err != nil && !os.IsNotExist(err) {
			return nil, nil, status.Errorf(codes.Internal, "remove socket %s: %v", addr, err)
		}
		cleanup = func() { _ = os.Remove(addr) }
	}

	l, err := net.Listen(proto, addr)
	return l, cleanup, err
}
