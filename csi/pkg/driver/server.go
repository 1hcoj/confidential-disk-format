// SPDX-License-Identifier: Apache-2.0

package driver

import (
	"net/http"
	"sync"

	endpoint "github.com/confidential-containers/confidential-disk-format/csi/internal/endpoint"
	"github.com/container-storage-interface/spec/lib/go/csi"
	"google.golang.org/grpc"
	"k8s.io/klog/v2"
)

type Driver struct {
	cfg    Config
	state  *volumeState
	loops  *loopTable
	grpc   *grpcServer
}

type loopTable struct {
	mu sync.Mutex
	// volumeID -> loop device
	devices map[string]string
}

func newLoopTable() *loopTable {
	return &loopTable{devices: make(map[string]string)}
}

func (t *loopTable) Set(volumeID, device string) {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.devices[volumeID] = device
}

func (t *loopTable) Get(volumeID string) (string, bool) {
	t.mu.Lock()
	defer t.mu.Unlock()
	d, ok := t.devices[volumeID]
	return d, ok
}

func (t *loopTable) Delete(volumeID string) {
	t.mu.Lock()
	defer t.mu.Unlock()
	delete(t.devices, volumeID)
}

type grpcServer struct {
	wg      sync.WaitGroup
	server  *grpc.Server
	cleanup func()
}

func New(cfg Config) (*Driver, error) {
	if cfg.StateDir == "" {
		cfg.StateDir = "/var/lib/cdf-csi/state"
	}
	if cfg.DefaultVolDir == "" {
		cfg.DefaultVolDir = DefaultVolumeDir
	}
	if cfg.VendorVersion == "" {
		cfg.VendorVersion = "dev"
	}

	st, err := newVolumeState(cfg.StateDir)
	if err != nil {
		return nil, err
	}

	return &Driver{
		cfg:   cfg,
		state: st,
		loops: newLoopTable(),
	}, nil
}

func (d *Driver) Run() error {
	go d.serveHealthz()

	d.grpc = &grpcServer{}
	d.grpc.wg.Add(1)
	go func() {
		defer d.grpc.wg.Done()
		if err := d.serveGRPC(d.grpc); err != nil {
			klog.Fatalf("grpc serve failed: %v", err)
		}
	}()
	d.grpc.wg.Wait()
	return nil
}

func (d *Driver) serveHealthz() {
	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ok"))
	})
	klog.Info("healthz listening on :9898")
	if err := http.ListenAndServe(":9898", mux); err != nil {
		klog.Errorf("healthz server: %v", err)
	}
}

func (d *Driver) serveGRPC(s *grpcServer) error {
	listener, cleanup, err := endpoint.Listen(d.cfg.Endpoint)
	if err != nil {
		return err
	}
	s.cleanup = cleanup

	opts := []grpc.ServerOption{}
	server := grpc.NewServer(opts...)
	s.server = server

	csi.RegisterIdentityServer(server, d)
	csi.RegisterControllerServer(server, d)
	csi.RegisterNodeServer(server, d)

	klog.Infof("CSI listening on %s", listener.Addr())
	return server.Serve(listener)
}

func (d *Driver) Stop() {
	if d.grpc != nil && d.grpc.server != nil {
		d.grpc.server.GracefulStop()
		if d.grpc.cleanup != nil {
			d.grpc.cleanup()
		}
	}
}

// Ensure Driver implements CSI services at compile time.
var (
	_ csi.IdentityServer   = (*Driver)(nil)
	_ csi.ControllerServer = (*Driver)(nil)
	_ csi.NodeServer       = (*Driver)(nil)
)
