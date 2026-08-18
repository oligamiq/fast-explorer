package main

/*
#include <stdlib.h>
*/
import "C"

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"encoding/xml"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
	"unsafe"

	"github.com/gofrs/flock"
	"golang.org/x/net/webdav"
	"tailscale.com/client/local"
	"tailscale.com/tailcfg"
	"tailscale.com/tsnet"
)

const (
	appProtocol      = "fast-explorer-tailnet/1"
	appPort          = 47891
	maxResponseBytes = 64 * 1024
	libraryVersion   = "tailscale-1.98.8"
)

type profileBridge struct {
	profileID string
	lifecycle sync.Mutex
	mu        sync.RWMutex

	srv           *tsnet.Server
	client        *local.Client
	stateLock     *flock.Flock
	stateDir      string
	listener      net.Listener
	httpServer    *http.Server
	authURL       string
	lastError     string
	hostname      string
	serviceReady  bool
	shareRoot     string
	webdavRoot    *os.Root
	webdavHandler *webdav.Handler

	taildriveGatewayListener net.Listener
	taildriveGatewayServer   *http.Server
	taildriveGatewayURL      string

	taildriveDevices   []taildriveDeviceInfo
	taildriveLastScan  time.Time
	taildriveScanBusy  bool
	taildriveScanError string
}

type bridgeManager struct {
	mu       sync.RWMutex
	profiles map[string]*profileBridge
	errors   map[string]string
}

var manager = bridgeManager{
	profiles: make(map[string]*profileBridge),
	errors:   make(map[string]string),
}

type taildriveTransferProgress struct {
	Phase      string `json:"phase"`
	BytesDone  int64  `json:"bytes_done"`
	BytesTotal int64  `json:"bytes_total"`
	ItemsDone  int64  `json:"items_done"`
	ItemsTotal int64  `json:"items_total"`
	Paused     bool   `json:"paused"`
	Cancelled  bool   `json:"cancelled"`
	Done       bool   `json:"done"`
	Error      string `json:"error,omitempty"`
}

type taildriveTransferControl struct {
	mu        sync.Mutex
	cond      *sync.Cond
	paused    bool
	cancelled bool
}

var taildriveTransferControls = struct {
	sync.RWMutex
	values map[string]*taildriveTransferControl
}{values: make(map[string]*taildriveTransferControl)}

var errTaildriveTransferCancelled = errors.New("transfer cancelled")

func prepareTaildriveTransferControl(id string) {
	if id == "" {
		return
	}
	control := &taildriveTransferControl{}
	control.cond = sync.NewCond(&control.mu)
	taildriveTransferControls.Lock()
	taildriveTransferControls.values[id] = control
	taildriveTransferControls.Unlock()
	updateTaildriveTransfer(id, func(progress *taildriveTransferProgress) {
		progress.Paused = false
		progress.Cancelled = false
	})
}

func taildriveTransferControlFor(id string) *taildriveTransferControl {
	taildriveTransferControls.RLock()
	control := taildriveTransferControls.values[id]
	taildriveTransferControls.RUnlock()
	return control
}

func controlTaildriveTransfer(id, action string) bool {
	if id == "" {
		return false
	}
	if action == "release" {
		releaseTaildriveTransferControl(id)
		return true
	}
	control := taildriveTransferControlFor(id)
	if action == "prepare" {
		prepareTaildriveTransferControl(id)
		return true
	}
	if control == nil {
		return false
	}
	control.mu.Lock()
	switch action {
	case "pause":
		if !control.cancelled {
			control.paused = true
		}
	case "resume":
		control.paused = false
		control.cond.Broadcast()
	case "cancel":
		control.cancelled = true
		control.paused = false
		control.cond.Broadcast()
	default:
		control.mu.Unlock()
		return false
	}
	paused := control.paused
	cancelled := control.cancelled
	control.mu.Unlock()
	updateTaildriveTransfer(id, func(progress *taildriveTransferProgress) {
		progress.Paused = paused
		progress.Cancelled = cancelled
		if cancelled {
			progress.Phase = "Cancelling"
		} else if paused {
			progress.Phase = "Paused"
		}
	})
	return true
}

func waitTaildriveTransferControl(id string) error {
	if id == "" {
		return nil
	}
	control := taildriveTransferControlFor(id)
	if control == nil {
		return nil
	}
	control.mu.Lock()
	defer control.mu.Unlock()
	for control.paused && !control.cancelled {
		control.cond.Wait()
	}
	if control.cancelled {
		return errTaildriveTransferCancelled
	}
	return nil
}

func releaseTaildriveTransferControl(id string) {
	if id == "" {
		return
	}
	taildriveTransferControls.Lock()
	delete(taildriveTransferControls.values, id)
	taildriveTransferControls.Unlock()
}

var taildriveTransfers = struct {
	sync.RWMutex
	values map[string]taildriveTransferProgress
}{values: make(map[string]taildriveTransferProgress)}

func setTaildriveTransferProgress(id string, progress taildriveTransferProgress) {
	if id == "" {
		return
	}
	taildriveTransfers.Lock()
	defer taildriveTransfers.Unlock()
	if _, exists := taildriveTransfers.values[id]; !exists && len(taildriveTransfers.values) >= 128 {
		for oldID := range taildriveTransfers.values {
			delete(taildriveTransfers.values, oldID)
			break
		}
	}
	taildriveTransfers.values[id] = progress
}

func updateTaildriveTransfer(id string, update func(*taildriveTransferProgress)) {
	if id == "" {
		return
	}
	taildriveTransfers.Lock()
	defer taildriveTransfers.Unlock()
	progress := taildriveTransfers.values[id]
	update(&progress)
	taildriveTransfers.values[id] = progress
}

func taildriveTransferSnapshot(id string) taildriveTransferProgress {
	taildriveTransfers.RLock()
	defer taildriveTransfers.RUnlock()
	return taildriveTransfers.values[id]
}

type peerInfo struct {
	ID       string   `json:"id"`
	HostName string   `json:"hostname"`
	DNSName  string   `json:"dns_name"`
	OS       string   `json:"os"`
	IPs      []string `json:"ips"`
	Online   bool     `json:"online"`
	Target   string   `json:"target"`
}

type taildriveDeviceInfo struct {
	ID       string   `json:"id"`
	HostName string   `json:"hostname"`
	DNSName  string   `json:"dns_name"`
	OS       string   `json:"os"`
	IPs      []string `json:"ips"`
	Online   bool     `json:"online"`
	Target   string   `json:"target"`
	Shares   []string `json:"shares"`
}

type taildriveCandidate struct {
	device      taildriveDeviceInfo
	peerAPIURLs []string
}

type taildriveProbeState uint8

const (
	taildriveProbeTransient taildriveProbeState = iota
	taildriveProbeAvailable
	taildriveProbeUnavailable
)

type taildriveProbeResult struct {
	id     string
	device taildriveDeviceInfo
	state  taildriveProbeState
}

type davMultiStatus struct {
	Responses []struct {
		Href string `xml:"href"`
	} `xml:"response"`
}

type statusPayload struct {
	Protocol            string                `json:"protocol"`
	LibraryVersion      string                `json:"library_version"`
	State               string                `json:"state"`
	AuthURL             string                `json:"auth_url,omitempty"`
	HostName            string                `json:"hostname,omitempty"`
	DNSName             string                `json:"dns_name,omitempty"`
	TailnetName         string                `json:"tailnet_name,omitempty"`
	MagicDNSSuffix      string                `json:"magic_dns_suffix,omitempty"`
	IPs                 []string              `json:"ips"`
	ServiceReady        bool                  `json:"service_ready"`
	WebDAVURL           string                `json:"webdav_url,omitempty"`
	Peers               []peerInfo            `json:"peers"`
	TaildriveDevices    []taildriveDeviceInfo `json:"taildrive_devices"`
	TaildriveGatewayURL string                `json:"taildrive_gateway_url,omitempty"`
	TaildriveScanning   bool                  `json:"taildrive_scanning"`
	TaildriveError      string                `json:"taildrive_error,omitempty"`
	Error               string                `json:"error,omitempty"`
}
type helloPayload struct {
	Protocol string   `json:"protocol"`
	HostName string   `json:"hostname"`
	DNSName  string   `json:"dns_name,omitempty"`
	IPs      []string `json:"ips"`
}

type pingPayload struct {
	OK        bool          `json:"ok"`
	Target    string        `json:"target"`
	LatencyMS int64         `json:"latency_ms,omitempty"`
	Remote    *helloPayload `json:"remote,omitempty"`
	Error     string        `json:"error,omitempty"`
}

func validProfileID(profileID string) bool {
	if profileID == "" || len(profileID) > 64 {
		return false
	}
	for _, ch := range profileID {
		if !((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
			(ch >= '0' && ch <= '9') || ch == '-' || ch == '_') {
			return false
		}
	}
	return true
}
func profileFor(profileID string, create bool) (*profileBridge, error) {
	if !validProfileID(profileID) {
		return nil, errors.New("invalid Tailscale profile ID")
	}
	manager.mu.RLock()
	profile := manager.profiles[profileID]
	manager.mu.RUnlock()
	if profile != nil || !create {
		return profile, nil
	}
	manager.mu.Lock()
	defer manager.mu.Unlock()
	if profile = manager.profiles[profileID]; profile == nil {
		profile = &profileBridge{profileID: profileID}
		manager.profiles[profileID] = profile
	}
	return profile, nil
}

func lockProfile(profileID string, create bool) (*profileBridge, error) {
	for {
		profile, err := profileFor(profileID, create)
		if err != nil || profile == nil {
			return profile, err
		}
		profile.lifecycle.Lock()
		manager.mu.RLock()
		current := manager.profiles[profileID] == profile
		manager.mu.RUnlock()
		if current {
			return profile, nil
		}
		profile.lifecycle.Unlock()
		if !create {
			return nil, nil
		}
	}
}

func setDetachedError(profileID string, err error) {
	manager.mu.Lock()
	defer manager.mu.Unlock()
	if err == nil {
		delete(manager.errors, profileID)
		return
	}
	if _, exists := manager.errors[profileID]; !exists && len(manager.errors) >= 64 {
		for oldID := range manager.errors {
			delete(manager.errors, oldID)
			break
		}
	}
	manager.errors[profileID] = err.Error()
}

func takeDetachedError(profileID string) string {
	manager.mu.Lock()
	defer manager.mu.Unlock()
	message := manager.errors[profileID]
	delete(manager.errors, profileID)
	return message
}

func (profile *profileBridge) setLastError(err error) {
	profile.mu.Lock()
	defer profile.mu.Unlock()
	if err == nil {
		profile.lastError = ""
	} else {
		profile.lastError = err.Error()
	}
}
func (profile *profileBridge) currentLastError() string {
	profile.mu.RLock()
	defer profile.mu.RUnlock()
	return profile.lastError
}

func setShareRoot(profileID, root string) error {
	profile, err := lockProfile(profileID, true)
	if err != nil {
		return err
	}
	defer profile.lifecycle.Unlock()
	absolute, err := filepath.Abs(root)
	if err != nil {
		return fmt.Errorf("resolve WebDAV root: %w", err)
	}
	profile.mu.RLock()
	alreadyConfigured := profile.shareRoot == absolute && profile.webdavRoot != nil
	running := profile.srv != nil
	profile.mu.RUnlock()
	if alreadyConfigured {
		return nil
	}
	if running {
		return errors.New("WebDAV root cannot change while Tailnet service is running")
	}
	rootHandle, err := os.OpenRoot(absolute)
	if err != nil {
		return fmt.Errorf("open WebDAV root: %w", err)
	}
	profile.mu.Lock()
	oldRoot := profile.webdavRoot
	profile.shareRoot = absolute
	profile.webdavRoot = rootHandle
	profile.webdavHandler = &webdav.Handler{
		Prefix:     "/dav/",
		FileSystem: newRootedWebDAVFS(rootHandle),
		LockSystem: webdav.NewMemLS(),
	}
	profile.mu.Unlock()
	if oldRoot != nil {
		_ = oldRoot.Close()
	}
	return nil
}

func (profile *profileBridge) userLogf(format string, args ...any) {
	url := extractURL(fmt.Sprintf(format, args...))
	if url == "" {
		return
	}
	profile.mu.Lock()
	profile.authURL = url
	profile.mu.Unlock()
}

func extractURL(message string) string {
	for _, field := range strings.Fields(message) {
		candidate := strings.Trim(field, " \t\r\n<>()[]{}\"',")
		if strings.HasPrefix(candidate, "https://") || strings.HasPrefix(candidate, "http://") {
			return candidate
		}
	}
	return ""
}

func stableHostname(stateDir string) (string, error) {
	if data, err := os.ReadFile(filepath.Join(stateDir, "hostname")); err == nil {
		if hostname := strings.TrimSpace(string(data)); hostname != "" {
			return hostname, nil
		}
	}
	path := filepath.Join(stateDir, "device-id")
	if data, err := os.ReadFile(path); err == nil {
		if id := strings.TrimSpace(string(data)); id != "" {
			if len(id) > 6 {
				id = id[:6]
			}
			return "fe-" + id, nil
		}
	}
	var random [4]byte
	if _, err := rand.Read(random[:]); err != nil {
		return "", err
	}
	id := hex.EncodeToString(random[:])
	if err := os.WriteFile(path, []byte(id+"\n"), 0o600); err != nil {
		return "", err
	}
	return "fe-" + id[:6], nil
}

func acquireStateLock(stateDir string) (*flock.Flock, error) {
	lock := flock.New(filepath.Join(stateDir, ".fast-explorer-tsnet.lock"))
	locked, err := lock.TryLock()
	if err != nil {
		_ = lock.Close()
		return nil, err
	}
	if !locked {
		_ = lock.Close()
		return nil, errors.New("Tailscale profile is already in use by another FastExplorer process")
	}
	return lock, nil
}

func releaseStateLock(lock *flock.Flock) {
	if lock == nil {
		return
	}
	_ = lock.Unlock()
	_ = lock.Close()
}

func startBridge(profileID, stateDir string) (retErr error) {
	profile, err := lockProfile(profileID, true)
	if err != nil {
		if validProfileID(profileID) {
			setDetachedError(profileID, err)
		}
		return err
	}
	defer func() {
		profile.setLastError(retErr)
		profile.lifecycle.Unlock()
	}()
	setDetachedError(profileID, nil)

	profile.mu.RLock()
	alreadyStarted := profile.srv != nil
	profile.mu.RUnlock()
	if alreadyStarted {
		return nil
	}
	if stateDir == "" {
		return errors.New("Tailscale state directory is empty")
	}
	if err := preparePlatformRuntime(stateDir); err != nil {
		return fmt.Errorf("prepare embedded Tailscale runtime: %w", err)
	}
	if err := os.MkdirAll(stateDir, 0o700); err != nil {
		return fmt.Errorf("create Tailscale state directory: %w", err)
	}
	if err := os.Chmod(stateDir, 0o700); err != nil && !errors.Is(err, os.ErrPermission) {
		return fmt.Errorf("secure Tailscale state directory: %w", err)
	}
	stateLock, err := acquireStateLock(stateDir)
	if err != nil {
		return err
	}
	hostname, err := stableHostname(stateDir)
	if err != nil {
		releaseStateLock(stateLock)
		return fmt.Errorf("create stable Tailscale hostname: %w", err)
	}

	srv := &tsnet.Server{
		Dir:      stateDir,
		Hostname: hostname,
		Port:     0,
		UserLogf: profile.userLogf,
	}
	if err := srv.Start(); err != nil {
		releaseStateLock(stateLock)
		return fmt.Errorf("start embedded Tailscale: %w", err)
	}
	client, err := srv.LocalClient()
	if err != nil {
		_ = srv.Close()
		releaseStateLock(stateLock)
		return fmt.Errorf("open embedded Tailscale local client: %w", err)
	}

	profile.mu.Lock()
	profile.srv = srv
	profile.client = client
	profile.stateLock = stateLock
	profile.stateDir = stateDir
	profile.hostname = hostname
	profile.lastError = ""
	profile.serviceReady = false
	profile.mu.Unlock()
	go serveFastExplorer(profile, srv, client)
	return nil
}

func serveFastExplorer(profile *profileBridge, srv *tsnet.Server, client *local.Client) {
	listener, err := srv.Listen("tcp", ":"+strconv.Itoa(appPort))
	if err != nil {
		profile.mu.RLock()
		stillCurrent := profile.srv == srv
		profile.mu.RUnlock()
		if stillCurrent {
			profile.setLastError(fmt.Errorf("start FastExplorer tailnet service: %w", err))
		}
		return
	}

	profile.mu.Lock()
	if profile.srv != srv {
		profile.mu.Unlock()
		_ = listener.Close()
		return
	}
	profile.listener = listener
	profile.serviceReady = true
	profile.mu.Unlock()

	mux := http.NewServeMux()
	mux.HandleFunc("/v1/hello", func(w http.ResponseWriter, r *http.Request) {
		handleHello(client, w, r)
	})
	mux.HandleFunc("/dav/", func(w http.ResponseWriter, r *http.Request) {
		if err := authorizeTailnetOwner(client, r.RemoteAddr); err != nil {
			http.Error(w, "forbidden", http.StatusForbidden)
			return
		}
		profile.mu.RLock()
		handler := profile.webdavHandler
		profile.mu.RUnlock()
		if handler == nil {
			http.Error(w, "WebDAV share is unavailable", http.StatusServiceUnavailable)
			return
		}
		handler.ServeHTTP(w, r)
	})
	server := &http.Server{
		Handler:           mux,
		ReadHeaderTimeout: 3 * time.Second,
		// WebDAV transfers may legitimately take minutes; keep only header and idle
		// deadlines so large reads/writes are not cut off mid-transfer.
		ReadTimeout:    0,
		WriteTimeout:   0,
		IdleTimeout:    60 * time.Second,
		MaxHeaderBytes: 8 * 1024,
	}

	profile.mu.Lock()
	if profile.srv == srv {
		profile.httpServer = server
	}
	profile.mu.Unlock()

	err = server.Serve(listener)
	if err != nil && !errors.Is(err, http.ErrServerClosed) && !errors.Is(err, net.ErrClosed) {
		profile.setLastError(fmt.Errorf("FastExplorer tailnet service stopped: %w", err))
	}
	profile.mu.Lock()
	if profile.srv == srv {
		profile.serviceReady = false
		profile.listener = nil
		profile.httpServer = nil
	}
	profile.mu.Unlock()
}
func authorizeTailnetPeer(client *local.Client, remoteAddr string) error {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	who, err := client.WhoIs(ctx, remoteAddr)
	if err != nil {
		return fmt.Errorf("identify Tailscale caller: %w", err)
	}
	if who == nil || who.Node == nil {
		return errors.New("Tailscale caller has no node identity")
	}
	return nil
}

func validateUserScopedOwner(
	remoteTagged bool,
	remoteNodeUser tailcfg.UserID,
	remoteProfileUser tailcfg.UserID,
	localTagged bool,
	localUser tailcfg.UserID,
) error {
	if remoteNodeUser.IsZero() || remoteProfileUser.IsZero() {
		return errors.New("Tailscale caller has no user identity")
	}
	if remoteTagged {
		return errors.New("tagged Tailscale callers cannot access user-scoped WebDAV")
	}
	if remoteNodeUser != remoteProfileUser {
		return errors.New("Tailscale caller returned inconsistent node/user identity")
	}
	if localUser.IsZero() {
		return errors.New("local Tailscale node has no owner identity")
	}
	if localTagged {
		return errors.New("tagged local Tailscale nodes cannot expose user-scoped WebDAV")
	}
	if remoteProfileUser != localUser {
		return errors.New("Tailscale caller owner does not match local owner")
	}
	return nil
}

func authorizeTailnetOwner(client *local.Client, remoteAddr string) error {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	who, err := client.WhoIs(ctx, remoteAddr)
	if err != nil {
		return fmt.Errorf("identify Tailscale caller: %w", err)
	}
	if who == nil || who.Node == nil || who.UserProfile == nil {
		return errors.New("Tailscale caller has no user identity")
	}
	status, err := client.StatusWithoutPeers(ctx)
	if err != nil {
		return fmt.Errorf("identify local Tailscale owner: %w", err)
	}
	if status.Self == nil {
		return errors.New("local Tailscale node has no owner identity")
	}
	return validateUserScopedOwner(
		who.Node.IsTagged(),
		who.Node.User,
		who.UserProfile.ID,
		status.Self.IsTagged(),
		status.Self.UserID,
	)
}

func handleHello(client *local.Client, w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if err := authorizeTailnetPeer(client, r.RemoteAddr); err != nil {
		http.Error(w, "forbidden", http.StatusForbidden)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
	defer cancel()
	status, err := client.Status(ctx)
	if err != nil {
		http.Error(w, "status unavailable", http.StatusServiceUnavailable)
		return
	}
	payload := helloPayload{Protocol: appProtocol, IPs: []string{}}
	if status.Self != nil {
		payload.HostName = status.Self.HostName
		payload.DNSName = strings.TrimSuffix(status.Self.DNSName, ".")
		for _, ip := range status.Self.TailscaleIPs {
			payload.IPs = append(payload.IPs, ip.String())
		}
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	_ = json.NewEncoder(w).Encode(payload)
}

func taildriveSharesFromMultiStatus(body []byte) ([]string, error) {
	var multi davMultiStatus
	if err := xml.Unmarshal(body, &multi); err != nil {
		return nil, fmt.Errorf("parse Taildrive WebDAV response: %w", err)
	}
	shares := make(map[string]struct{})
	for _, response := range multi.Responses {
		href := strings.TrimSpace(response.Href)
		if href == "" {
			continue
		}
		path := href
		if parsed, err := url.Parse(href); err == nil && parsed.EscapedPath() != "" {
			path = parsed.EscapedPath()
		}
		if index := strings.Index(path, "/v0/drive/"); index >= 0 {
			path = path[index+len("/v0/drive/"):]
		}
		path = strings.Trim(path, "/")
		if path == "" {
			continue
		}
		name := strings.SplitN(path, "/", 2)[0]
		if decoded, err := url.PathUnescape(name); err == nil {
			name = decoded
		}
		name = strings.TrimSpace(name)
		if name != "" {
			shares[name] = struct{}{}
		}
	}
	result := make([]string, 0, len(shares))
	for name := range shares {
		result = append(result, name)
	}
	sort.Strings(result)
	return result, nil
}

type taildriveDialFunc func(context.Context, string, string) (net.Conn, error)

func probeTaildriveDevice(dial taildriveDialFunc, candidate taildriveCandidate) taildriveProbeResult {
	const propfindBody = `<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>`
	definitiveUnavailable := false
	for _, base := range candidate.peerAPIURLs {
		base = strings.TrimRight(base, "/")
		if base == "" {
			continue
		}
		ctx, cancel := context.WithTimeout(context.Background(), 4*time.Second)
		request, err := http.NewRequestWithContext(ctx, "PROPFIND", base+"/v0/drive/", strings.NewReader(propfindBody))
		if err != nil {
			cancel()
			continue
		}
		request.Header.Set("Depth", "1")
		request.Header.Set("Content-Type", "application/xml; charset=utf-8")
		transport := &http.Transport{
			DialContext:       dial,
			DisableKeepAlives: true,
			Proxy:             nil,
		}
		client := &http.Client{Transport: transport, Timeout: 4 * time.Second}
		response, err := client.Do(request)
		if err != nil {
			transport.CloseIdleConnections()
			cancel()
			continue
		}
		limited := io.LimitReader(response.Body, 1024*1024+1)
		body, readErr := io.ReadAll(limited)
		_ = response.Body.Close()
		transport.CloseIdleConnections()
		cancel()
		if response.StatusCode == http.StatusForbidden || response.StatusCode == http.StatusNotFound {
			definitiveUnavailable = true
			continue
		}
		if readErr != nil || response.StatusCode != http.StatusMultiStatus || len(body) > 1024*1024 {
			continue
		}
		shares, err := taildriveSharesFromMultiStatus(body)
		if err != nil {
			continue
		}
		device := candidate.device
		device.Shares = shares
		return taildriveProbeResult{id: device.ID, device: device, state: taildriveProbeAvailable}
	}
	state := taildriveProbeTransient
	if definitiveUnavailable {
		state = taildriveProbeUnavailable
	}
	return taildriveProbeResult{id: candidate.device.ID, state: state}
}

func maybeStartTaildriveScan(profile *profileBridge, srv *tsnet.Server, candidates []taildriveCandidate) {
	profile.mu.Lock()
	if profile.srv != srv || profile.taildriveScanBusy || (!profile.taildriveLastScan.IsZero() && time.Since(profile.taildriveLastScan) < 30*time.Second) {
		profile.mu.Unlock()
		return
	}
	profile.taildriveScanBusy = true
	profile.taildriveScanError = ""
	profile.mu.Unlock()

	go func() {
		results := make(chan taildriveProbeResult, len(candidates))
		semaphore := make(chan struct{}, 4)
		var wait sync.WaitGroup
		for _, candidate := range candidates {
			wait.Add(1)
			go func(candidate taildriveCandidate) {
				defer wait.Done()
				semaphore <- struct{}{}
				defer func() { <-semaphore }()
				results <- probeTaildriveDevice(srv.Dial, candidate)
			}(candidate)
		}
		wait.Wait()
		close(results)
		probes := make([]taildriveProbeResult, 0, len(candidates))
		for result := range results {
			probes = append(probes, result)
		}
		profile.mu.Lock()
		if profile.srv == srv {
			byID := make(map[string]taildriveDeviceInfo, len(profile.taildriveDevices)+len(probes))
			for _, device := range profile.taildriveDevices {
				byID[device.ID] = device
			}
			for _, probe := range probes {
				switch probe.state {
				case taildriveProbeAvailable:
					byID[probe.id] = probe.device
				case taildriveProbeUnavailable:
					delete(byID, probe.id)
				case taildriveProbeTransient:
					// Keep the last successful registration across temporary timeouts.
				}
			}
			devices := make([]taildriveDeviceInfo, 0, len(byID))
			for _, device := range byID {
				devices = append(devices, device)
			}
			sort.Slice(devices, func(i, j int) bool {
				left := strings.ToLower(devices[i].HostName)
				right := strings.ToLower(devices[j].HostName)
				if left == right {
					return devices[i].Target < devices[j].Target
				}
				return left < right
			})
			profile.taildriveDevices = devices
			profile.taildriveLastScan = time.Now()
			profile.taildriveScanError = ""
			profile.taildriveScanBusy = false
		}
		profile.mu.Unlock()
	}()
}

func statusSnapshot(profileID string) statusPayload {
	payload := statusPayload{
		Protocol:         appProtocol,
		LibraryVersion:   libraryVersion,
		State:            "NotStarted",
		IPs:              []string{},
		Peers:            []peerInfo{},
		TaildriveDevices: []taildriveDeviceInfo{},
	}
	profile, err := profileFor(profileID, false)
	if err != nil {
		payload.State = "Error"
		payload.Error = err.Error()
		return payload
	}
	if profile == nil {
		return payload
	}
	profile.mu.RLock()
	srv := profile.srv
	client := profile.client
	payload.AuthURL = profile.authURL
	payload.HostName = profile.hostname
	payload.ServiceReady = profile.serviceReady
	payload.TaildriveGatewayURL = profile.taildriveGatewayURL
	payload.Error = profile.lastError
	profile.mu.RUnlock()

	if srv == nil || client == nil {
		return payload
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	status, err := client.Status(ctx)
	if err != nil {
		payload.State = "Error"
		payload.Error = err.Error()
		return payload
	}
	payload.State = status.BackendState
	if status.CurrentTailnet != nil {
		payload.TailnetName = status.CurrentTailnet.Name
		payload.MagicDNSSuffix = status.CurrentTailnet.MagicDNSSuffix
	}
	payload.AuthURL = status.AuthURL
	profile.mu.Lock()
	if profile.srv == srv {
		profile.authURL = status.AuthURL
	}
	profile.mu.Unlock()
	if status.Self == nil {
		return payload
	}
	payload.HostName = status.Self.HostName
	payload.DNSName = strings.TrimSuffix(status.Self.DNSName, ".")
	for _, ip := range status.Self.TailscaleIPs {
		payload.IPs = append(payload.IPs, ip.String())
	}
	webdavHost := payload.DNSName
	if webdavHost == "" && len(payload.IPs) > 0 {
		webdavHost = payload.IPs[0]
	}
	profile.mu.RLock()
	hasWebDAV := profile.webdavHandler != nil
	profile.mu.RUnlock()
	if hasWebDAV && webdavHost != "" {
		payload.WebDAVURL = "http://" + net.JoinHostPort(webdavHost, strconv.Itoa(appPort)) + "/dav/"
	}
	candidates := make([]taildriveCandidate, 0, len(status.Peer))
	currentPeers := make(map[string]peerInfo, len(status.Peer))
	for _, peer := range status.Peer {
		if peer == nil {
			continue
		}
		info := peerInfo{
			ID:       string(peer.ID),
			HostName: peer.HostName,
			DNSName:  strings.TrimSuffix(peer.DNSName, "."),
			OS:       peer.OS,
			Online:   peer.Online,
			IPs:      []string{},
		}
		for _, ip := range peer.TailscaleIPs {
			info.IPs = append(info.IPs, ip.String())
		}
		if info.DNSName != "" {
			info.Target = info.DNSName
		} else if len(info.IPs) > 0 {
			info.Target = info.IPs[0]
		}
		if info.Target != "" {
			currentPeers[info.ID] = info
			payload.Peers = append(payload.Peers, info)
		}
		if peer.Online && len(peer.PeerAPIURL) > 0 && info.Target != "" {
			candidates = append(candidates, taildriveCandidate{
				device: taildriveDeviceInfo{
					ID:       info.ID,
					HostName: info.HostName,
					DNSName:  info.DNSName,
					OS:       info.OS,
					IPs:      append([]string(nil), info.IPs...),
					Online:   info.Online,
					Target:   info.Target,
					Shares:   []string{},
				},
				peerAPIURLs: append([]string(nil), peer.PeerAPIURL...),
			})
		}
	}
	sort.Slice(payload.Peers, func(i, j int) bool {
		left := strings.ToLower(payload.Peers[i].HostName)
		right := strings.ToLower(payload.Peers[j].HostName)
		if left == right {
			return payload.Peers[i].Target < payload.Peers[j].Target
		}
		return left < right
	})
	if payload.State == "Running" {
		maybeStartTaildriveScan(profile, srv, candidates)
	}
	profile.mu.RLock()
	payload.TaildriveDevices = append([]taildriveDeviceInfo{}, profile.taildriveDevices...)
	payload.TaildriveScanning = profile.taildriveScanBusy
	payload.TaildriveError = profile.taildriveScanError
	profile.mu.RUnlock()
	for index := range payload.TaildriveDevices {
		device := &payload.TaildriveDevices[index]
		peer, ok := currentPeers[device.ID]
		if !ok {
			device.Online = false
			continue
		}
		device.HostName = peer.HostName
		device.DNSName = peer.DNSName
		device.OS = peer.OS
		device.IPs = peer.IPs
		device.Online = peer.Online
		device.Target = peer.Target
	}
	payload.Error = profile.currentLastError()
	return payload
}

func pingTarget(profileID, target string) pingPayload {
	result := pingPayload{Target: target}
	profile, err := profileFor(profileID, false)
	if err != nil {
		result.Error = err.Error()
		return result
	}
	if profile == nil {
		result.Error = "embedded Tailscale profile is not started"
		return result
	}
	profile.mu.RLock()
	srv := profile.srv
	profile.mu.RUnlock()
	if srv == nil {
		result.Error = "embedded Tailscale profile is not started"
		return result
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	transport := &http.Transport{
		DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
			return srv.Dial(ctx, network, address)
		},
		DisableKeepAlives: true,
	}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 5 * time.Second}
	url := "http://" + net.JoinHostPort(target, strconv.Itoa(appPort)) + "/v1/hello"
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		result.Error = err.Error()
		return result
	}
	started := time.Now()
	response, err := client.Do(request)
	if err != nil {
		result.Error = err.Error()
		return result
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		result.Error = fmt.Sprintf("peer returned HTTP %d", response.StatusCode)
		return result
	}
	limited := io.LimitReader(response.Body, maxResponseBytes+1)
	body, err := io.ReadAll(limited)
	if err != nil {
		result.Error = err.Error()
		return result
	}
	if len(body) > maxResponseBytes {
		result.Error = "peer response exceeded size limit"
		return result
	}
	var hello helloPayload
	if err := json.Unmarshal(body, &hello); err != nil {
		result.Error = "invalid FastExplorer peer response"
		return result
	}
	if hello.Protocol != appProtocol {
		result.Error = "peer is not a compatible FastExplorer service"
		return result
	}
	result.OK = true
	result.LatencyMS = time.Since(started).Milliseconds()
	result.Remote = &hello
	return result
}

func removeProfileIfSame(profileID string, profile *profileBridge) {
	manager.mu.Lock()
	defer manager.mu.Unlock()
	if manager.profiles[profileID] == profile {
		delete(manager.profiles, profileID)
	}
}

func stopBridge(profileID string) error {
	profile, err := lockProfile(profileID, false)
	if err != nil || profile == nil {
		return err
	}
	defer profile.lifecycle.Unlock()
	stopBridgeLocked(profile)
	removeProfileIfSame(profileID, profile)
	return nil
}
func stopBridgeLocked(profile *profileBridge) {
	profile.mu.Lock()
	srv := profile.srv
	server := profile.httpServer
	listener := profile.listener
	taildriveGatewayServer := profile.taildriveGatewayServer
	taildriveGatewayListener := profile.taildriveGatewayListener
	stateLock := profile.stateLock
	webdavRoot := profile.webdavRoot
	profile.srv = nil
	profile.client = nil
	profile.listener = nil
	profile.httpServer = nil
	profile.taildriveGatewayServer = nil
	profile.taildriveGatewayListener = nil
	profile.taildriveGatewayURL = ""
	profile.stateLock = nil
	profile.stateDir = ""
	profile.authURL = ""
	profile.hostname = ""
	profile.serviceReady = false
	profile.shareRoot = ""
	profile.webdavRoot = nil
	profile.webdavHandler = nil
	profile.taildriveDevices = nil
	profile.taildriveLastScan = time.Time{}
	profile.taildriveScanBusy = false
	profile.taildriveScanError = ""
	profile.mu.Unlock()

	if server != nil {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		_ = server.Shutdown(ctx)
		cancel()
	}
	if listener != nil {
		_ = listener.Close()
	}
	if taildriveGatewayServer != nil {
		ctx, cancel := context.WithTimeout(context.Background(), time.Second)
		_ = taildriveGatewayServer.Shutdown(ctx)
		cancel()
	}
	if taildriveGatewayListener != nil {
		_ = taildriveGatewayListener.Close()
	}
	if srv != nil {
		_ = srv.Close()
	}
	if webdavRoot != nil {
		_ = webdavRoot.Close()
	}
	releaseStateLock(stateLock)
}

func logoutBridge(profileID string) error {
	profile, err := lockProfile(profileID, false)
	if err != nil || profile == nil {
		return err
	}
	defer profile.lifecycle.Unlock()

	profile.mu.RLock()
	client := profile.client
	profile.mu.RUnlock()
	if client == nil {
		stopBridgeLocked(profile)
		removeProfileIfSame(profileID, profile)
		return nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	err = client.Logout(ctx)
	cancel()
	stopBridgeLocked(profile)
	removeProfileIfSame(profileID, profile)
	return err
}

type taildriveListPayload struct {
	Entries []taildriveBrowserEntry `json:"entries"`
	Error   string                  `json:"error,omitempty"`
}

func taildriveDownload(profileID, deviceID, share, remotePath, destination string) error {
	return taildriveDownloadWithProgress(profileID, deviceID, share, remotePath, destination, "")
}

func taildriveList(profileID, deviceID, share, remotePath string) taildriveListPayload {
	profile, err := profileFor(profileID, false)
	if err != nil || profile == nil {
		if err == nil {
			err = errors.New("Tailscale profile is not running")
		}
		return taildriveListPayload{Error: err.Error()}
	}
	normalizedPath, err := normalizeTaildriveBrowserPath(remotePath)
	if err != nil {
		return taildriveListPayload{Error: err.Error()}
	}
	profile.mu.RLock()
	srv := profile.srv
	client := profile.client
	devices := append([]taildriveDeviceInfo(nil), profile.taildriveDevices...)
	profile.mu.RUnlock()
	if srv == nil || client == nil {
		return taildriveListPayload{Error: "Tailscale profile is not connected"}
	}
	if !taildriveShareKnown(devices, deviceID, share) {
		return taildriveListPayload{Error: "Taildrive share is not currently available"}
	}
	entries, err := listTaildriveDirectory(profile, srv, client, deviceID, share, normalizedPath)
	if err != nil {
		return taildriveListPayload{Error: err.Error()}
	}
	return taildriveListPayload{Entries: entries}
}

func jsonCString(value any) *C.char {
	data, err := json.Marshal(value)
	if err != nil {
		data = []byte(`{"error":"failed to encode bridge response"}`)
	}
	return C.CString(string(data))
}

func goString(value *C.char, name string) (string, error) {
	if value == nil {
		return "", fmt.Errorf("%s is null", name)
	}
	return C.GoString(value), nil
}

//export FE_TS_SetAndroidInterfacesJSON
func FE_TS_SetAndroidInterfacesJSON(raw *C.char) C.int {
	value, err := goString(raw, "Android network interfaces JSON")
	if err == nil {
		err = setAndroidInterfacesJSON(value)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_SetShareRoot
func FE_TS_SetShareRoot(profileID, root *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	path, err := goString(root, "WebDAV root")
	if err == nil {
		err = setShareRoot(profile, path)
	}
	if err != nil {
		setDetachedError(profile, err)
		return 0
	}
	return 1
}

//export FE_TS_Start
func FE_TS_Start(profileID, stateDir *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	dir, err := goString(stateDir, "Tailscale state directory")
	if err == nil {
		err = startBridge(profile, dir)
	} else if validProfileID(profile) {
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_StatusJSON
func FE_TS_StatusJSON(profileID *C.char) *C.char {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return jsonCString(statusPayload{Protocol: appProtocol, State: "Error", Error: err.Error()})
	}
	return jsonCString(statusSnapshot(profile))
}

//export FE_TS_TaildriveListJSON
func FE_TS_TaildriveListJSON(profileID, deviceID, share, remotePath *C.char) *C.char {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return jsonCString(taildriveListPayload{Error: err.Error()})
	}
	device, err := goString(deviceID, "Taildrive device ID")
	if err != nil {
		return jsonCString(taildriveListPayload{Error: err.Error()})
	}
	shareName, err := goString(share, "Taildrive share")
	if err != nil {
		return jsonCString(taildriveListPayload{Error: err.Error()})
	}
	path, err := goString(remotePath, "Taildrive path")
	if err != nil {
		return jsonCString(taildriveListPayload{Error: err.Error()})
	}
	return jsonCString(taildriveList(profile, device, shareName, path))
}

//export FE_TS_TaildriveDownload
func FE_TS_TaildriveDownload(profileID, deviceID, share, remotePath, destination *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	device, err := goString(deviceID, "Taildrive device ID")
	if err == nil {
		var shareName, path, target string
		shareName, err = goString(share, "Taildrive share")
		if err == nil {
			path, err = goString(remotePath, "Taildrive path")
		}
		if err == nil {
			target, err = goString(destination, "download destination")
		}
		if err == nil {
			err = taildriveDownload(profile, device, shareName, path, target)
		}
	}
	if bridge, _ := profileFor(profile, false); bridge != nil {
		bridge.setLastError(err)
	} else {
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_TaildriveDownloadToFD
func FE_TS_TaildriveDownloadToFD(profileID, deviceID, share, remotePath *C.char, destinationFD C.int) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	device, err := goString(deviceID, "Taildrive device ID")
	if err == nil {
		var shareName, path string
		shareName, err = goString(share, "Taildrive share")
		if err == nil {
			path, err = goString(remotePath, "Taildrive path")
		}
		if err == nil {
			err = taildriveDownloadToFD(profile, device, shareName, path, int(destinationFD))
		}
	}
	if bridge, _ := profileFor(profile, false); bridge != nil {
		bridge.setLastError(err)
	} else {
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_TaildriveUpload
func FE_TS_TaildriveUpload(profileID, deviceID, share, remotePath, source *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	device, err := goString(deviceID, "Taildrive device ID")
	if err == nil {
		var shareName, path, sourcePath string
		shareName, err = goString(share, "Taildrive share")
		if err == nil {
			path, err = goString(remotePath, "Taildrive path")
		}
		if err == nil {
			sourcePath, err = goString(source, "upload source")
		}
		if err == nil {
			err = taildriveUpload(profile, device, shareName, path, sourcePath)
		}
	}
	if bridge, _ := profileFor(profile, false); bridge != nil {
		bridge.setLastError(err)
	} else {
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_TaildriveDownloadProgress
func FE_TS_TaildriveDownloadProgress(profileID, deviceID, share, remotePath, destination, transferID *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	device, err := goString(deviceID, "Taildrive device ID")
	if err == nil {
		var shareName, path, target, transfer string
		shareName, err = goString(share, "Taildrive share")
		if err == nil {
			path, err = goString(remotePath, "Taildrive path")
		}
		if err == nil {
			target, err = goString(destination, "download destination")
		}
		if err == nil {
			transfer, err = goString(transferID, "Taildrive transfer ID")
		}
		if err == nil {
			err = taildriveDownloadWithProgress(profile, device, shareName, path, target, transfer)
		}
	}
	if bridge, _ := profileFor(profile, false); bridge != nil {
		bridge.setLastError(err)
	} else {
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_TaildriveUploadProgress
func FE_TS_TaildriveUploadProgress(profileID, deviceID, share, remotePath, source, transferID *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	device, err := goString(deviceID, "Taildrive device ID")
	if err == nil {
		var shareName, path, sourcePath, transfer string
		shareName, err = goString(share, "Taildrive share")
		if err == nil {
			path, err = goString(remotePath, "Taildrive path")
		}
		if err == nil {
			sourcePath, err = goString(source, "upload source")
		}
		if err == nil {
			transfer, err = goString(transferID, "Taildrive transfer ID")
		}
		if err == nil {
			err = taildriveUploadWithProgress(profile, device, shareName, path, sourcePath, transfer)
		}
	}
	if bridge, _ := profileFor(profile, false); bridge != nil {
		bridge.setLastError(err)
	} else {
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_TaildriveUploadReplaceProgress
func FE_TS_TaildriveUploadReplaceProgress(profileID, deviceID, share, remotePath, source, transferID *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	device, err := goString(deviceID, "Taildrive device ID")
	if err == nil {
		var shareName, path, sourcePath, transfer string
		shareName, err = goString(share, "Taildrive share")
		if err == nil {
			path, err = goString(remotePath, "Taildrive path")
		}
		if err == nil {
			sourcePath, err = goString(source, "upload source")
		}
		if err == nil {
			transfer, err = goString(transferID, "Taildrive transfer ID")
		}
		if err == nil {
			err = taildriveUploadReplaceWithProgress(profile, device, shareName, path, sourcePath, transfer)
		}
	}
	if bridge, _ := profileFor(profile, false); bridge != nil {
		bridge.setLastError(err)
	} else {
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_TaildriveProgressJSON
func FE_TS_TaildriveProgressJSON(transferID *C.char) *C.char {
	transfer, err := goString(transferID, "Taildrive transfer ID")
	if err != nil {
		return jsonCString(taildriveTransferProgress{Done: true, Error: err.Error()})
	}
	return jsonCString(taildriveTransferSnapshot(transfer))
}

//export FE_TS_TaildriveControl
func FE_TS_TaildriveControl(transferID, action *C.char) C.int {
	transfer, err := goString(transferID, "Taildrive transfer ID")
	if err != nil {
		return 0
	}
	controlAction, err := goString(action, "Taildrive transfer action")
	if err != nil {
		return 0
	}
	if controlTaildriveTransfer(transfer, controlAction) {
		return 1
	}
	return 0
}

//export FE_TS_TaildriveMkdir
func FE_TS_TaildriveMkdir(profileID, deviceID, share, remotePath *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	device, err := goString(deviceID, "Taildrive device ID")
	if err == nil {
		var shareName, path string
		shareName, err = goString(share, "Taildrive share")
		if err == nil {
			path, err = goString(remotePath, "Taildrive path")
		}
		if err == nil {
			err = taildriveMkdir(profile, device, shareName, path)
		}
	}
	if bridge, _ := profileFor(profile, false); bridge != nil {
		bridge.setLastError(err)
	} else {
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_TaildriveDelete
func FE_TS_TaildriveDelete(profileID, deviceID, share, remotePath *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	device, err := goString(deviceID, "Taildrive device ID")
	if err == nil {
		var shareName, path string
		shareName, err = goString(share, "Taildrive share")
		if err == nil {
			path, err = goString(remotePath, "Taildrive path")
		}
		if err == nil {
			err = taildriveDelete(profile, device, shareName, path)
		}
	}
	if bridge, _ := profileFor(profile, false); bridge != nil {
		bridge.setLastError(err)
	} else {
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_TaildriveRename
func FE_TS_TaildriveRename(profileID, deviceID, share, remotePath, newName *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return 0
	}
	device, err := goString(deviceID, "Taildrive device ID")
	if err == nil {
		var shareName, path, targetName string
		shareName, err = goString(share, "Taildrive share")
		if err == nil {
			path, err = goString(remotePath, "Taildrive path")
		}
		if err == nil {
			targetName, err = goString(newName, "Taildrive new name")
		}
		if err == nil {
			err = taildriveRename(profile, device, shareName, path, targetName)
		}
	}
	if bridge, _ := profileFor(profile, false); bridge != nil {
		bridge.setLastError(err)
	} else {
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_PingJSON
func FE_TS_PingJSON(profileID, target *C.char) *C.char {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return jsonCString(pingPayload{Error: err.Error()})
	}
	peer, err := goString(target, "peer target")
	if err != nil {
		return jsonCString(pingPayload{Error: err.Error()})
	}
	return jsonCString(pingTarget(profile, peer))
}

//export FE_TS_Logout
func FE_TS_Logout(profileID *C.char) C.int {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err == nil {
		err = logoutBridge(profile)
		setDetachedError(profile, err)
	}
	if err != nil {
		return 0
	}
	return 1
}

//export FE_TS_Stop
func FE_TS_Stop(profileID *C.char) {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err == nil {
		_ = stopBridge(profile)
	}
}

//export FE_TS_LastError
func FE_TS_LastError(profileID *C.char) *C.char {
	profile, err := goString(profileID, "Tailscale profile ID")
	if err != nil {
		return C.CString(err.Error())
	}
	bridge, _ := profileFor(profile, false)
	if bridge == nil {
		return C.CString(takeDetachedError(profile))
	}
	return C.CString(bridge.currentLastError())
}

//export FE_TS_Free
func FE_TS_Free(value *C.char) {
	if value != nil {
		C.free(unsafe.Pointer(value))
	}
}

func main() {}
