// SPDX-License-Identifier: Apache-2.0

package driver

import (
	"context"

	"github.com/container-storage-interface/spec/lib/go/csi"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func (d *Driver) GetPluginInfo(ctx context.Context, req *csi.GetPluginInfoRequest) (*csi.GetPluginInfoResponse, error) {
	if DriverName == "" {
		return nil, status.Error(codes.Unavailable, "driver name not configured")
	}
	return &csi.GetPluginInfoResponse{
		Name:          DriverName,
		VendorVersion: d.cfg.VendorVersion,
	}, nil
}

func (d *Driver) Probe(ctx context.Context, req *csi.ProbeRequest) (*csi.ProbeResponse, error) {
	return &csi.ProbeResponse{}, nil
}

func (d *Driver) GetPluginCapabilities(ctx context.Context, req *csi.GetPluginCapabilitiesRequest) (*csi.GetPluginCapabilitiesResponse, error) {
	caps := []*csi.PluginCapability{
		{
			Type: &csi.PluginCapability_Service_{
				Service: &csi.PluginCapability_Service{
					Type: csi.PluginCapability_Service_CONTROLLER_SERVICE,
				},
			},
		},
		{
			Type: &csi.PluginCapability_Service_{
				Service: &csi.PluginCapability_Service{
					Type: csi.PluginCapability_Service_VOLUME_ACCESSIBILITY_CONSTRAINTS,
				},
			},
		},
	}
	return &csi.GetPluginCapabilitiesResponse{Capabilities: caps}, nil
}
