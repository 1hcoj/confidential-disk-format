// SPDX-License-Identifier: Apache-2.0

package driver

import (
	"context"
	"os"
	"path/filepath"

	cdfpkg "github.com/confidential-containers/confidential-disk-format/csi/pkg/cdf"
	dv "github.com/confidential-containers/confidential-disk-format/csi/pkg/directvolume"
	"github.com/container-storage-interface/spec/lib/go/csi"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"k8s.io/klog/v2"
)

func (d *Driver) NodeStageVolume(ctx context.Context, req *csi.NodeStageVolumeRequest) (*csi.NodeStageVolumeResponse, error) {
	volumeID := req.GetVolumeId()
	if volumeID == "" {
		return nil, status.Error(codes.InvalidArgument, "volume id missing")
	}
	if req.GetStagingTargetPath() == "" {
		return nil, status.Error(codes.InvalidArgument, "staging target path missing")
	}

	volCtx := req.GetVolumeContext()
	if err := requireBareMetal(volCtx); err != nil {
		return nil, err
	}

	imagePath, err := imagePathFromContext(volCtx)
	if err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "%v", err)
	}
	if _, err := os.Stat(imagePath); err != nil {
		return nil, status.Errorf(codes.NotFound, "host image %s: %v", imagePath, err)
	}

	if dev, ok := d.loops.Get(volumeID); ok {
		if _, err := os.Stat(dev); err == nil {
			klog.Infof("NodeStageVolume volume %s already staged on %s", volumeID, dev)
			return &csi.NodeStageVolumeResponse{}, nil
		}
		d.loops.Delete(volumeID)
	}

	device, err := cdfpkg.LosetupFindShow(imagePath)
	if err != nil {
		return nil, status.Errorf(codes.Internal, "losetup: %v", err)
	}
	d.loops.Set(volumeID, device)

	if err := os.MkdirAll(req.GetStagingTargetPath(), 0o750); err != nil {
		return nil, status.Errorf(codes.Internal, "mkdir staging path: %v", err)
	}

	klog.Infof("NodeStageVolume id=%s device=%s image=%s", volumeID, device, imagePath)
	return &csi.NodeStageVolumeResponse{}, nil
}

func (d *Driver) NodeUnstageVolume(ctx context.Context, req *csi.NodeUnstageVolumeRequest) (*csi.NodeUnstageVolumeResponse, error) {
	volumeID := req.GetVolumeId()
	if volumeID == "" {
		return nil, status.Error(codes.InvalidArgument, "volume id missing")
	}

	if dev, ok := d.loops.Get(volumeID); ok {
		if err := cdfpkg.LosetupDetach(dev); err != nil {
			klog.Warningf("losetup detach %s: %v", dev, err)
		}
		d.loops.Delete(volumeID)
	}

	staging := req.GetStagingTargetPath()
	if staging != "" {
		_ = os.RemoveAll(staging)
	}
	return &csi.NodeUnstageVolumeResponse{}, nil
}

func (d *Driver) NodePublishVolume(ctx context.Context, req *csi.NodePublishVolumeRequest) (*csi.NodePublishVolumeResponse, error) {
	volumeID := req.GetVolumeId()
	targetPath := req.GetTargetPath()
	if volumeID == "" {
		return nil, status.Error(codes.InvalidArgument, "volume id missing")
	}
	if targetPath == "" {
		return nil, status.Error(codes.InvalidArgument, "target path missing")
	}
	if req.GetVolumeCapability().GetMount() == nil {
		return nil, status.Error(codes.InvalidArgument, "mount capability required")
	}

	volCtx := req.GetVolumeContext()
	if err := requireBareMetal(volCtx); err != nil {
		return nil, err
	}

	device, ok := d.loops.Get(volumeID)
	if !ok || device == "" {
		return nil, status.Errorf(codes.FailedPrecondition, "volume %q not staged", volumeID)
	}

	if err := os.MkdirAll(targetPath, 0o750); err != nil {
		return nil, status.Errorf(codes.Internal, "mkdir target path: %v", err)
	}

	meta := publishMetadata(volCtx)
	info := dv.MountInfo{
		VolumeType: "cdf",
		Device:     device,
		FsType:     fstypeFrom(volCtx),
		Metadata:   meta,
		Options:    []string{"defaults"},
	}
	if err := dv.WriteMountInfo(targetPath, info); err != nil {
		return nil, status.Errorf(codes.Internal, "write mountInfo.json: %v", err)
	}

	klog.Infof("NodePublishVolume id=%s target=%s device=%s", volumeID, targetPath, device)
	return &csi.NodePublishVolumeResponse{}, nil
}

func (d *Driver) NodeUnpublishVolume(ctx context.Context, req *csi.NodeUnpublishVolumeRequest) (*csi.NodeUnpublishVolumeResponse, error) {
	targetPath := req.GetTargetPath()
	if targetPath == "" {
		return nil, status.Error(codes.InvalidArgument, "target path missing")
	}

	if err := dv.Remove(targetPath); err != nil {
		return nil, status.Errorf(codes.Internal, "remove direct volume: %v", err)
	}
	if err := os.RemoveAll(targetPath); err != nil {
		return nil, status.Errorf(codes.Internal, "remove target path: %v", err)
	}
	return &csi.NodeUnpublishVolumeResponse{}, nil
}

func (d *Driver) NodeGetInfo(ctx context.Context, req *csi.NodeGetInfoRequest) (*csi.NodeGetInfoResponse, error) {
	return &csi.NodeGetInfoResponse{
		NodeId: d.cfg.NodeID,
		AccessibleTopology: &csi.Topology{
			Segments: map[string]string{TopologyKeyNode: d.cfg.NodeID},
		},
	}, nil
}

func (d *Driver) NodeGetCapabilities(ctx context.Context, req *csi.NodeGetCapabilitiesRequest) (*csi.NodeGetCapabilitiesResponse, error) {
	return &csi.NodeGetCapabilitiesResponse{
		Capabilities: []*csi.NodeServiceCapability{
			{
				Type: &csi.NodeServiceCapability_Rpc{
					Rpc: &csi.NodeServiceCapability_RPC{
						Type: csi.NodeServiceCapability_RPC_STAGE_UNSTAGE_VOLUME,
					},
				},
			},
		},
	}, nil
}

func (d *Driver) NodeGetVolumeStats(ctx context.Context, req *csi.NodeGetVolumeStatsRequest) (*csi.NodeGetVolumeStatsResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

func (d *Driver) NodeExpandVolume(ctx context.Context, req *csi.NodeExpandVolumeRequest) (*csi.NodeExpandVolumeResponse, error) {
	return nil, status.Error(codes.Unimplemented, "")
}

// StagingMarkerPath is used by tests; kubelet uses staging_target_path directly.
func StagingMarkerPath(stagingTargetPath, volumeID string) string {
	return filepath.Join(stagingTargetPath, volumeID)
}
