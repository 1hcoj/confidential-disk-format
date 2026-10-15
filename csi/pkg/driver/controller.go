// SPDX-License-Identifier: Apache-2.0

package driver

import (
	"context"
	"os"
	"path/filepath"
	"strings"

	cdfpkg "github.com/confidential-containers/confidential-disk-format/csi/pkg/cdf"
	"github.com/container-storage-interface/spec/lib/go/csi"
	"github.com/google/uuid"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"k8s.io/klog/v2"
)

func (d *Driver) CreateVolume(ctx context.Context, req *csi.CreateVolumeRequest) (*csi.CreateVolumeResponse, error) {
	if err := d.validateControllerCapability(csi.ControllerServiceCapability_RPC_CREATE_DELETE_VOLUME); err != nil {
		return nil, err
	}
	name := req.GetName()
	if name == "" {
		return nil, status.Error(codes.InvalidArgument, "name missing")
	}
	if req.GetCapacityRange() == nil {
		return nil, status.Error(codes.InvalidArgument, "capacity range missing")
	}
	size := req.GetCapacityRange().GetRequiredBytes()
	if size <= 0 {
		return nil, status.Error(codes.InvalidArgument, "invalid capacity")
	}

	params := mergeVolumeContext(req.GetParameters())
	if err := requireBareMetal(params); err != nil {
		return nil, err
	}
	if backend := strings.TrimSpace(params[ParamBackend]); backend != "" && backend != BackendFile {
		return nil, status.Errorf(codes.InvalidArgument, "unsupported %s=%q", ParamBackend, backend)
	}

	if existing, err := d.state.GetByName(name); err == nil {
		if existing.SizeBytes < size {
			return nil, status.Errorf(codes.AlreadyExists, "volume %q exists with smaller size", name)
		}
		return d.createVolumeResponse(existing, params), nil
	}

	volDir := volumeDirFrom(params, d.cfg.DefaultVolDir)
	if err := os.MkdirAll(volDir, 0o750); err != nil {
		return nil, status.Errorf(codes.Internal, "mkdir volume dir: %v", err)
	}

	// The guest needs a locator to fetch K_root and initialize the blank disk.
	if cdfpkg.ResolveKbsKeyID(params) == "" {
		return nil, status.Errorf(codes.InvalidArgument, "missing key locator: set %q or %q",
			cdfpkg.ParamKbsKeyID, cdfpkg.AnnotationKbsKeyID)
	}

	volumeID := uuid.NewString()
	hostPath := filepath.Join(volDir, volumeID+".img")

	profile := profileFrom(params)
	if err := cdfpkg.AllocateBlankImage(hostPath, size); err != nil {
		return nil, status.Errorf(codes.Internal, "allocate volume: %v", err)
	}

	volCtx := mergeVolumeContext(params)
	volCtx[ParamHostPath] = hostPath
	volCtx[ParamBackend] = BackendFile
	if volCtx[ParamFstype] == "" {
		volCtx[ParamFstype] = DefaultFstype
	}
	if volCtx[ParamProfile] == "" {
		volCtx[ParamProfile] = profile
	}
	if kbs := cdfpkg.ResolveKbsKeyID(params); kbs != "" {
		volCtx[cdfpkg.ParamKbsKeyID] = kbs
	}

	rec := &volumeRecord{
		VolumeID:  volumeID,
		Name:      name,
		HostPath:  hostPath,
		SizeBytes: size,
		Context:   volCtx,
	}
	if err := d.state.Put(rec); err != nil {
		_ = os.Remove(hostPath)
		return nil, status.Errorf(codes.Internal, "persist state: %v", err)
	}

	klog.Infof("CreateVolume name=%s id=%s path=%s", name, volumeID, hostPath)
	return d.createVolumeResponse(rec, volCtx), nil
}

func (d *Driver) createVolumeResponse(rec *volumeRecord, volCtx map[string]string) *csi.CreateVolumeResponse {
	topology := []*csi.Topology{{
		Segments: map[string]string{TopologyKeyNode: d.cfg.NodeID},
	}}
	return &csi.CreateVolumeResponse{
		Volume: &csi.Volume{
			VolumeId:           rec.VolumeID,
			CapacityBytes:      rec.SizeBytes,
			VolumeContext:      volCtx,
			AccessibleTopology: topology,
		},
	}
}

func (d *Driver) DeleteVolume(ctx context.Context, req *csi.DeleteVolumeRequest) (*csi.DeleteVolumeResponse, error) {
	if err := d.validateControllerCapability(csi.ControllerServiceCapability_RPC_CREATE_DELETE_VOLUME); err != nil {
		return nil, err
	}
	volumeID := req.GetVolumeId()
	if volumeID == "" {
		return nil, status.Error(codes.InvalidArgument, "volume id missing")
	}

	rec, err := d.state.GetByID(volumeID)
	if err != nil {
		if os.IsNotExist(err) {
			return &csi.DeleteVolumeResponse{}, nil
		}
		return nil, status.Errorf(codes.Internal, "load state: %v", err)
	}

	if rec.HostPath != "" {
		if err := os.Remove(rec.HostPath); err != nil && !os.IsNotExist(err) {
			return nil, status.Errorf(codes.Internal, "remove %s: %v", rec.HostPath, err)
		}
	}
	_ = d.state.Delete(volumeID)
	klog.Infof("DeleteVolume id=%s", volumeID)
	return &csi.DeleteVolumeResponse{}, nil
}

func (d *Driver) ControllerPublishVolume(ctx context.Context, req *csi.ControllerPublishVolumeRequest) (*csi.ControllerPublishVolumeResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) ControllerUnpublishVolume(ctx context.Context, req *csi.ControllerUnpublishVolumeRequest) (*csi.ControllerUnpublishVolumeResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) ValidateVolumeCapabilities(ctx context.Context, req *csi.ValidateVolumeCapabilitiesRequest) (*csi.ValidateVolumeCapabilitiesResponse, error) {
	if req.GetVolumeId() == "" {
		return nil, status.Error(codes.InvalidArgument, "volume id missing")
	}
	for _, cap := range req.GetVolumeCapabilities() {
		if cap.GetMount() == nil {
			return nil, status.Error(codes.InvalidArgument, "only mount access type supported")
		}
	}
	return &csi.ValidateVolumeCapabilitiesResponse{
		Confirmed: &csi.ValidateVolumeCapabilitiesResponse_Confirmed{
			VolumeCapabilities: req.GetVolumeCapabilities(),
		},
	}, nil
}

func (d *Driver) ListVolumes(ctx context.Context, req *csi.ListVolumesRequest) (*csi.ListVolumesResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) GetCapacity(ctx context.Context, req *csi.GetCapacityRequest) (*csi.GetCapacityResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) ControllerGetCapabilities(ctx context.Context, req *csi.ControllerGetCapabilitiesRequest) (*csi.ControllerGetCapabilitiesResponse, error) {
	caps := []csi.ControllerServiceCapability_RPC_Type{
		csi.ControllerServiceCapability_RPC_CREATE_DELETE_VOLUME,
	}
	var out []*csi.ControllerServiceCapability
	for _, c := range caps {
		out = append(out, &csi.ControllerServiceCapability{
			Type: &csi.ControllerServiceCapability_Rpc{
				Rpc: &csi.ControllerServiceCapability_RPC{Type: c},
			},
		})
	}
	return &csi.ControllerGetCapabilitiesResponse{Capabilities: out}, nil
}

func (d *Driver) CreateSnapshot(ctx context.Context, req *csi.CreateSnapshotRequest) (*csi.CreateSnapshotResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) DeleteSnapshot(ctx context.Context, req *csi.DeleteSnapshotRequest) (*csi.DeleteSnapshotResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) ListSnapshots(ctx context.Context, req *csi.ListSnapshotsRequest) (*csi.ListSnapshotsResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) ControllerExpandVolume(ctx context.Context, req *csi.ControllerExpandVolumeRequest) (*csi.ControllerExpandVolumeResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) ControllerGetVolume(ctx context.Context, req *csi.ControllerGetVolumeRequest) (*csi.ControllerGetVolumeResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) ControllerModifyVolume(ctx context.Context, req *csi.ControllerModifyVolumeRequest) (*csi.ControllerModifyVolumeResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) validateControllerCapability(cap csi.ControllerServiceCapability_RPC_Type) error {
	switch cap {
	case csi.ControllerServiceCapability_RPC_CREATE_DELETE_VOLUME:
		return nil
	default:
		return status.Errorf(codes.InvalidArgument, "unsupported capability %v", cap)
	}
}
