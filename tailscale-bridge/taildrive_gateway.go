package main

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/hex"
	"encoding/xml"
	"errors"
	"fmt"
	"html"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	pathpkg "path"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"tailscale.com/client/local"
	"tailscale.com/tsnet"
)

const taildriveGatewayMaxListing = 2 * 1024 * 1024

type taildriveListMultiStatus struct {
	Responses []struct {
		Href      string `xml:"href"`
		Propstats []struct {
			Status string `xml:"status"`
			Prop   struct {
				DisplayName   string `xml:"displayname"`
				ContentLength string `xml:"getcontentlength"`
				LastModified  string `xml:"getlastmodified"`
				ResourceType  struct {
					Collection *struct{} `xml:"collection"`
				} `xml:"resourcetype"`
			} `xml:"prop"`
		} `xml:"propstat"`
	} `xml:"response"`
}

type taildriveBrowserEntry struct {
	Name      string `json:"name"`
	Path      string `json:"path"`
	Directory bool   `json:"directory"`
	Size      string `json:"size"`
	Modified  string `json:"modified"`
}

func startTaildriveGateway(profile *profileBridge, srv *tsnet.Server, client *local.Client) error {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return fmt.Errorf("start local Taildrive browser: %w", err)
	}
	var secretBytes [36]byte
	if _, err := rand.Read(secretBytes[:]); err != nil {
		_ = listener.Close()
		return fmt.Errorf("create Taildrive browser secrets: %w", err)
	}
	token := hex.EncodeToString(secretBytes[:18])
	csrfToken := hex.EncodeToString(secretBytes[18:])
	prefix := "/taildrive/" + token + "/"
	expectedHost := listener.Addr().String()
	mux := http.NewServeMux()
	mux.HandleFunc(prefix, func(w http.ResponseWriter, r *http.Request) {
		if !taildriveGatewayRequestAllowed(r, expectedHost) {
			http.Error(w, "forbidden", http.StatusForbidden)
			return
		}
		if r.Method == http.MethodPost && !taildriveCSRFValid(r.URL.Query().Get("csrf"), csrfToken) {
			http.Error(w, "forbidden", http.StatusForbidden)
			return
		}
		taildriveGatewayHandler(profile, srv, client, prefix, csrfToken, w, r)
	})
	server := &http.Server{
		Handler:           mux,
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       0,
		WriteTimeout:      0,
		IdleTimeout:       90 * time.Second,
		MaxHeaderBytes:    16 * 1024,
	}
	gatewayURL := "http://" + listener.Addr().String() + prefix

	profile.mu.Lock()
	if profile.srv != srv {
		profile.mu.Unlock()
		_ = listener.Close()
		return net.ErrClosed
	}
	profile.taildriveGatewayListener = listener
	profile.taildriveGatewayServer = server
	profile.taildriveGatewayURL = gatewayURL
	profile.mu.Unlock()

	go func() {
		err := server.Serve(listener)
		if err != nil && !errors.Is(err, http.ErrServerClosed) && !errors.Is(err, net.ErrClosed) {
			profile.mu.Lock()
			if profile.srv == srv {
				profile.taildriveScanError = "Taildrive browser stopped: " + err.Error()
			}
			profile.mu.Unlock()
		}
	}()
	return nil
}

func taildriveGatewayRequestAllowed(r *http.Request, expectedHost string) bool {
	if !strings.EqualFold(r.Host, expectedHost) {
		return false
	}
	if r.Method != http.MethodPost {
		return true
	}
	origin := strings.TrimSpace(r.Header.Get("Origin"))
	return strings.EqualFold(origin, "http://"+expectedHost)
}

func taildriveCSRFValid(provided, expected string) bool {
	return len(provided) == len(expected) && subtle.ConstantTimeCompare([]byte(provided), []byte(expected)) == 1
}

func validTaildriveLeafName(name string) bool {
	name = strings.TrimSpace(name)
	return name != "" && name != "." && name != ".." && !strings.ContainsAny(name, "/\\\x00")
}

func taildriveGatewayHandler(profile *profileBridge, srv *tsnet.Server, client *local.Client, prefix, csrfToken string, w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("Referrer-Policy", "no-referrer")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("X-Frame-Options", "DENY")
	w.Header().Set("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'")
	profile.mu.RLock()
	current := profile.srv == srv
	devices := append([]taildriveDeviceInfo(nil), profile.taildriveDevices...)
	profile.mu.RUnlock()
	if !current {
		http.Error(w, "Tailnet connection is no longer active", http.StatusServiceUnavailable)
		return
	}

	deviceID := r.URL.Query().Get("device")
	share := r.URL.Query().Get("share")
	remotePath, err := normalizeTaildriveBrowserPath(r.URL.Query().Get("path"))
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	if deviceID == "" || share == "" {
		if r.Method != http.MethodGet {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}
		renderTaildriveGatewayIndex(w, prefix, devices)
		return
	}
	if !taildriveShareKnown(devices, deviceID, share) {
		http.Error(w, "Taildrive share is not currently available", http.StatusNotFound)
		return
	}

	if r.Method == http.MethodPost {
		handleTaildriveBrowserPost(profile, srv, client, prefix, deviceID, share, remotePath, w, r)
		return
	}
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}

	isDir, err := taildriveRemoteIsDirectory(profile, srv, client, deviceID, share, remotePath)
	if err != nil {
		http.Error(w, "Taildrive lookup failed: "+err.Error(), http.StatusBadGateway)
		return
	}
	if !isDir {
		streamTaildriveFile(profile, srv, client, deviceID, share, remotePath, w, r)
		return
	}
	entries, err := listTaildriveDirectory(profile, srv, client, deviceID, share, remotePath)
	if err != nil {
		http.Error(w, "Taildrive listing failed: "+err.Error(), http.StatusBadGateway)
		return
	}
	renderTaildriveDirectory(w, prefix, csrfToken, deviceID, share, remotePath, entries)
}

func normalizeTaildriveBrowserPath(value string) (string, error) {
	value = strings.ReplaceAll(value, "\\", "/")
	clean := pathpkg.Clean("/" + strings.TrimSpace(value))
	if clean == "/" || clean == "." {
		return "", nil
	}
	if strings.Contains(clean, "\x00") {
		return "", errors.New("invalid Taildrive path")
	}
	return strings.TrimPrefix(clean, "/"), nil
}

func taildriveShareKnown(devices []taildriveDeviceInfo, deviceID, share string) bool {
	for _, device := range devices {
		if device.ID != deviceID || !device.Online {
			continue
		}
		for _, candidate := range device.Shares {
			if candidate == share {
				return true
			}
		}
	}
	return false
}

func taildrivePeerAPIURLs(client *local.Client, deviceID string) ([]string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	status, err := client.Status(ctx)
	if err != nil {
		return nil, err
	}
	for _, peer := range status.Peer {
		if peer != nil && string(peer.ID) == deviceID && peer.Online {
			return append([]string(nil), peer.PeerAPIURL...), nil
		}
	}
	return nil, errors.New("Taildrive device is offline")
}

func taildriveRemotePath(share, remotePath string, directory bool) string {
	parts := []string{url.PathEscape(share)}
	for _, part := range strings.Split(strings.Trim(remotePath, "/"), "/") {
		if part != "" {
			parts = append(parts, url.PathEscape(part))
		}
	}
	result := "/v0/drive/" + strings.Join(parts, "/")
	if directory && !strings.HasSuffix(result, "/") {
		result += "/"
	}
	return result
}

const taildriveIOIdleTimeout = 5 * time.Minute

type taildriveIdleConn struct {
	net.Conn
}

func (conn *taildriveIdleConn) Read(buffer []byte) (int, error) {
	if err := conn.Conn.SetReadDeadline(time.Now().Add(taildriveIOIdleTimeout)); err != nil {
		return 0, err
	}
	return conn.Conn.Read(buffer)
}

func (conn *taildriveIdleConn) Write(buffer []byte) (int, error) {
	if err := conn.Conn.SetWriteDeadline(time.Now().Add(taildriveIOIdleTimeout)); err != nil {
		return 0, err
	}
	return conn.Conn.Write(buffer)
}

func taildriveHTTPTransport(srv *tsnet.Server) *http.Transport {
	return &http.Transport{
		DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
			conn, err := srv.Dial(ctx, network, address)
			if err != nil {
				return nil, err
			}
			return &taildriveIdleConn{Conn: conn}, nil
		},
		Proxy:             nil,
		DisableKeepAlives: true,
	}
}

func doTaildriveRequest(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, method, remoteURLPath string, body io.Reader, headers http.Header) (*http.Response, error) {
	profile.mu.RLock()
	current := profile.srv == srv
	profile.mu.RUnlock()
	if !current {
		return nil, net.ErrClosed
	}
	bases, err := taildrivePeerAPIURLs(client, deviceID)
	if err != nil {
		return nil, err
	}
	var lastErr error
	var seeker io.ReadSeeker
	if body != nil {
		seeker, _ = body.(io.ReadSeeker)
	}
	for index, base := range bases {
		if index > 0 && body != nil {
			if seeker == nil {
				break
			}
			if _, err := seeker.Seek(0, io.SeekStart); err != nil {
				break
			}
		}
		var ctx context.Context
		var cancel context.CancelFunc
		if method == http.MethodGet || method == http.MethodPut {
			ctx, cancel = context.WithCancel(context.Background())
		} else {
			ctx, cancel = context.WithTimeout(context.Background(), 30*time.Second)
		}
		req, err := http.NewRequestWithContext(ctx, method, strings.TrimRight(base, "/")+remoteURLPath, body)
		if err != nil {
			cancel()
			return nil, err
		}
		for key, values := range headers {
			for _, value := range values {
				req.Header.Add(key, value)
			}
		}
		if value := req.Header.Get("Content-Length"); value != "" {
			if length, parseErr := strconv.ParseInt(value, 10, 64); parseErr == nil {
				req.ContentLength = length
				req.Header.Del("Content-Length")
			}
		}
		transport := taildriveHTTPTransport(srv)
		response, err := (&http.Client{Transport: transport}).Do(req)
		if err == nil {
			response.Body = &taildriveResponseBody{ReadCloser: response.Body, cancel: cancel, transport: transport}
			return response, nil
		}
		transport.CloseIdleConnections()
		cancel()
		lastErr = err
	}
	if lastErr == nil {
		lastErr = errors.New("Taildrive peer API is unavailable")
	}
	return nil, lastErr
}

func doTaildriveMoveRequest(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, sourceURLPath, destinationURLPath string, overwrite bool) (*http.Response, error) {
	profile.mu.RLock()
	current := profile.srv == srv
	profile.mu.RUnlock()
	if !current {
		return nil, net.ErrClosed
	}
	bases, err := taildrivePeerAPIURLs(client, deviceID)
	if err != nil {
		return nil, err
	}
	var lastErr error
	for _, base := range bases {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		base = strings.TrimRight(base, "/")
		req, err := http.NewRequestWithContext(ctx, "MOVE", base+sourceURLPath, nil)
		if err != nil {
			cancel()
			return nil, err
		}
		req.Header.Set("Destination", base+destinationURLPath)
		if overwrite {
			req.Header.Set("Overwrite", "T")
		} else {
			req.Header.Set("Overwrite", "F")
		}
		transport := taildriveHTTPTransport(srv)
		response, err := (&http.Client{Transport: transport}).Do(req)
		if err == nil {
			response.Body = &taildriveResponseBody{ReadCloser: response.Body, cancel: cancel, transport: transport}
			return response, nil
		}
		transport.CloseIdleConnections()
		cancel()
		lastErr = err
	}
	if lastErr == nil {
		lastErr = errors.New("Taildrive peer API is unavailable")
	}
	return nil, lastErr
}

type taildriveRewindableReader struct {
	io.ReadSeeker
}

type taildriveProgressReadSeeker struct {
	io.ReadSeeker
	transferID string
	base       int64
	total      int64
	position   int64
}

func (reader *taildriveProgressReadSeeker) Read(buffer []byte) (int, error) {
	if err := waitTaildriveTransferControl(reader.transferID); err != nil {
		return 0, err
	}
	n, err := reader.ReadSeeker.Read(buffer)
	reader.position += int64(n)
	updateTaildriveTransfer(reader.transferID, func(progress *taildriveTransferProgress) {
		progress.BytesDone = reader.base + reader.position
		progress.BytesTotal = reader.total
	})
	return n, err
}

func (reader *taildriveProgressReadSeeker) Seek(offset int64, whence int) (int64, error) {
	position, err := reader.ReadSeeker.Seek(offset, whence)
	if err == nil {
		reader.position = position
		updateTaildriveTransfer(reader.transferID, func(progress *taildriveTransferProgress) {
			progress.BytesDone = reader.base + position
			progress.BytesTotal = reader.total
		})
	}
	return position, err
}

type taildriveProgressWriter struct {
	writer     io.Writer
	transferID string
	base       int64
	total      int64
	written    int64
}

func (writer *taildriveProgressWriter) Write(buffer []byte) (int, error) {
	if err := waitTaildriveTransferControl(writer.transferID); err != nil {
		return 0, err
	}
	n, err := writer.writer.Write(buffer)
	writer.written += int64(n)
	updateTaildriveTransfer(writer.transferID, func(progress *taildriveTransferProgress) {
		progress.BytesDone = writer.base + writer.written
		progress.BytesTotal = writer.total
	})
	return n, err
}

type taildriveResponseBody struct {
	io.ReadCloser
	cancel    context.CancelFunc
	transport *http.Transport
}

func (body *taildriveResponseBody) Close() error {
	err := body.ReadCloser.Close()
	body.transport.CloseIdleConnections()
	body.cancel()
	return err
}

func taildriveRemoteIsDirectory(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, share, remotePath string) (bool, error) {
	headers := make(http.Header)
	headers.Set("Depth", "0")
	headers.Set("Content-Type", "application/xml; charset=utf-8")
	response, err := doTaildriveRequest(profile, srv, client, deviceID, "PROPFIND", taildriveRemotePath(share, remotePath, true), strings.NewReader(`<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/></d:prop></d:propfind>`), headers)
	if err != nil {
		return false, err
	}
	defer response.Body.Close()
	if response.StatusCode == http.StatusNotFound {
		return false, errors.New("path not found")
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, taildriveGatewayMaxListing+1))
	if err != nil || len(body) > taildriveGatewayMaxListing {
		return false, errors.New("invalid Taildrive metadata response")
	}
	if response.StatusCode != http.StatusMultiStatus {
		return false, fmt.Errorf("Taildrive returned %s", response.Status)
	}
	var multi taildriveListMultiStatus
	if err := xml.Unmarshal(body, &multi); err != nil {
		return false, err
	}
	for _, item := range multi.Responses {
		for _, propstat := range item.Propstats {
			if propstat.Prop.ResourceType.Collection != nil {
				return true, nil
			}
		}
	}
	return false, nil
}

func normalizeTaildriveHrefPath(href string) string {
	parsed, err := url.Parse(href)
	if err == nil && parsed.Path != "" {
		href = parsed.Path
	}
	if decoded, err := url.PathUnescape(href); err == nil {
		href = decoded
	}
	return strings.TrimSuffix(pathpkg.Clean("/"+strings.TrimSpace(href)), "/")
}

func isTaildriveSelfHref(href, share, remotePath string) bool {
	responsePath := normalizeTaildriveHrefPath(href)
	logicalPath := normalizeTaildriveHrefPath(pathpkg.Join("/", share, remotePath))
	peerAPIPath := normalizeTaildriveHrefPath(taildriveRemotePath(share, remotePath, true))
	return responsePath == logicalPath ||
		responsePath == peerAPIPath ||
		strings.HasSuffix(responsePath, peerAPIPath)
}

func listTaildriveDirectory(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, share, remotePath string) ([]taildriveBrowserEntry, error) {
	headers := make(http.Header)
	headers.Set("Depth", "1")
	headers.Set("Content-Type", "application/xml; charset=utf-8")
	bodyXML := `<?xml version="1.0"?><d:propfind xmlns:d="DAV:"><d:prop><d:displayname/><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>`
	response, err := doTaildriveRequest(profile, srv, client, deviceID, "PROPFIND", taildriveRemotePath(share, remotePath, true), strings.NewReader(bodyXML), headers)
	if err != nil {
		return nil, err
	}
	defer response.Body.Close()
	body, err := io.ReadAll(io.LimitReader(response.Body, taildriveGatewayMaxListing+1))
	if err != nil || len(body) > taildriveGatewayMaxListing {
		return nil, errors.New("Taildrive directory listing is too large")
	}
	if response.StatusCode != http.StatusMultiStatus {
		return nil, fmt.Errorf("Taildrive returned %s", response.Status)
	}
	var multi taildriveListMultiStatus
	if err := xml.Unmarshal(body, &multi); err != nil {
		return nil, err
	}
	entries := make([]taildriveBrowserEntry, 0, len(multi.Responses))
	for _, item := range multi.Responses {
		// WebDAV Depth: 1 includes the requested directory itself. Taildrive
		// proxies do not always use the same href prefix, so compare the logical
		// resource as well as the PeerAPI form before treating a response as a child.
		if isTaildriveSelfHref(item.Href, share, remotePath) {
			continue
		}
		var name, size, modified string
		isDir := false
		for _, propstat := range item.Propstats {
			if !strings.Contains(propstat.Status, " 200 ") {
				continue
			}
			if name == "" {
				name = strings.TrimSpace(propstat.Prop.DisplayName)
			}
			if size == "" {
				size = strings.TrimSpace(propstat.Prop.ContentLength)
			}
			if modified == "" {
				modified = strings.TrimSpace(propstat.Prop.LastModified)
			}
			isDir = isDir || propstat.Prop.ResourceType.Collection != nil
		}
		if name == "" {
			parsed, err := url.Parse(item.Href)
			if err == nil {
				name = pathpkg.Base(strings.TrimSuffix(parsed.Path, "/"))
				if decoded, err := url.PathUnescape(name); err == nil {
					name = decoded
				}
			}
		}
		if name == "" || name == "." || name == "/" {
			continue
		}
		childPath := pathpkg.Join(remotePath, name)
		if childPath == strings.Trim(remotePath, "/") {
			continue
		}
		entries = append(entries, taildriveBrowserEntry{Name: name, Path: childPath, Directory: isDir, Size: size, Modified: modified})
	}
	return entries, nil
}

func taildriveActiveContext(profileID, deviceID, share string) (*profileBridge, *tsnet.Server, *local.Client, error) {
	profile, err := profileFor(profileID, false)
	if err != nil || profile == nil {
		if err == nil {
			err = errors.New("Tailscale profile is not running")
		}
		return nil, nil, nil, err
	}
	profile.mu.RLock()
	srv := profile.srv
	client := profile.client
	devices := append([]taildriveDeviceInfo(nil), profile.taildriveDevices...)
	profile.mu.RUnlock()
	if srv == nil || client == nil {
		return nil, nil, nil, errors.New("Tailscale profile is not connected")
	}
	if !taildriveShareKnown(devices, deviceID, share) {
		return nil, nil, nil, errors.New("Taildrive share is not currently available")
	}
	return profile, srv, client, nil
}

func finishTaildriveMutation(response *http.Response, action string) error {
	if response == nil {
		return fmt.Errorf("%s failed: empty Taildrive response", action)
	}
	_, _ = io.Copy(io.Discard, response.Body)
	_ = response.Body.Close()
	if response.StatusCode == http.StatusMultiStatus {
		return fmt.Errorf("%s incomplete: %s", action, response.Status)
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return fmt.Errorf("%s rejected: %s", action, response.Status)
	}
	return nil
}

func taildriveTemporarySibling(remotePath string) (string, error) {
	var random [16]byte
	if _, err := rand.Read(random[:]); err != nil {
		return "", fmt.Errorf("create Taildrive temporary name: %w", err)
	}
	parent := pathpkg.Dir(remotePath)
	if parent == "." {
		parent = ""
	}
	return pathpkg.Join(parent, ".fastexplorer-upload-"+hex.EncodeToString(random[:])), nil
}

func putTaildriveFile(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, share, remotePath, source, transferID string, baseBytes, totalBytes int64) error {
	file, err := os.Open(source)
	if err != nil {
		return fmt.Errorf("open upload source: %w", err)
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return fmt.Errorf("stat upload source: %w", err)
	}
	if !info.Mode().IsRegular() {
		return errors.New("Taildrive upload source is not a regular file")
	}
	headers := make(http.Header)
	headers.Set("Content-Length", strconv.FormatInt(info.Size(), 10))
	reader := &taildriveProgressReadSeeker{
		ReadSeeker: file,
		transferID: transferID,
		base:       baseBytes,
		total:      totalBytes,
	}
	response, err := doTaildriveRequest(profile, srv, client, deviceID, http.MethodPut, taildriveRemotePath(share, remotePath, false), reader, headers)
	if err != nil {
		return err
	}
	return finishTaildriveMutation(response, "Taildrive upload")
}

func mkdirTaildrivePath(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, share, remotePath string) error {
	response, err := doTaildriveRequest(profile, srv, client, deviceID, "MKCOL", taildriveRemotePath(share, remotePath, true), nil, nil)
	if err != nil {
		return err
	}
	return finishTaildriveMutation(response, "Taildrive create folder")
}

func cleanupTaildriveTemporary(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, share, remotePath string, directory bool) {
	response, err := doTaildriveRequest(profile, srv, client, deviceID, http.MethodDelete, taildriveRemotePath(share, remotePath, directory), nil, nil)
	if err == nil && response != nil {
		_, _ = io.Copy(io.Discard, response.Body)
		_ = response.Body.Close()
	}
}

func publishTaildriveFile(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, share, remotePath, source, transferID string, baseBytes, totalBytes int64, overwrite bool) error {
	temporaryPath, err := taildriveTemporarySibling(remotePath)
	if err != nil {
		return err
	}
	if err := putTaildriveFile(profile, srv, client, deviceID, share, temporaryPath, source, transferID, baseBytes, totalBytes); err != nil {
		cleanupTaildriveTemporary(profile, srv, client, deviceID, share, temporaryPath, false)
		return err
	}
	response, err := doTaildriveMoveRequest(
		profile,
		srv,
		client,
		deviceID,
		taildriveRemotePath(share, temporaryPath, false),
		taildriveRemotePath(share, remotePath, false),
		overwrite,
	)
	if err != nil {
		cleanupTaildriveTemporary(profile, srv, client, deviceID, share, temporaryPath, false)
		return err
	}
	if err := finishTaildriveMutation(response, "Taildrive publish file"); err != nil {
		cleanupTaildriveTemporary(profile, srv, client, deviceID, share, temporaryPath, false)
		return err
	}
	return nil
}

type taildriveDownloadEntry struct {
	RemotePath string
	Relative   string
	Directory  bool
	Size       int64
	SizeKnown  bool
}

func collectTaildriveDownloadTree(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, share, rootPath string) ([]taildriveDownloadEntry, int64, bool, error) {
	const maxEntries = 100000
	entries := make([]taildriveDownloadEntry, 0)
	var totalBytes int64
	allSizesKnown := true
	var walk func(string, string) error
	walk = func(remotePath, relative string) error {
		children, err := listTaildriveDirectory(profile, srv, client, deviceID, share, remotePath)
		if err != nil {
			return err
		}
		for _, child := range children {
			if len(entries) >= maxEntries {
				return errors.New("Taildrive folder contains too many entries to copy safely")
			}
			if !validTaildriveLeafName(child.Name) {
				return fmt.Errorf("invalid Taildrive filename: %s", child.Name)
			}
			childRelative := pathpkg.Join(relative, child.Name)
			entry := taildriveDownloadEntry{
				RemotePath: child.Path,
				Relative:   childRelative,
				Directory:  child.Directory,
			}
			if !child.Directory {
				if size, parseErr := strconv.ParseInt(child.Size, 10, 64); parseErr == nil && size >= 0 {
					entry.Size = size
					entry.SizeKnown = true
					totalBytes += size
				} else {
					allSizesKnown = false
				}
			}
			entries = append(entries, entry)
			if child.Directory {
				if err := walk(child.Path, childRelative); err != nil {
					return err
				}
			}
		}
		return nil
	}
	if err := walk(rootPath, ""); err != nil {
		return nil, 0, false, err
	}
	return entries, totalBytes, allSizesKnown, nil
}

func downloadTaildriveFileTo(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, share, remotePath, destination, transferID string, baseBytes, totalBytes int64, discoverTotal bool) (int64, error) {
	response, err := doTaildriveRequest(profile, srv, client, deviceID, http.MethodGet, taildriveRemotePath(share, remotePath, false), nil, nil)
	if err != nil {
		return 0, err
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		_, _ = io.Copy(io.Discard, response.Body)
		return 0, fmt.Errorf("Taildrive download rejected: %s", response.Status)
	}
	if discoverTotal && totalBytes == 0 && response.ContentLength > 0 {
		totalBytes = response.ContentLength
		updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
			progress.BytesTotal = totalBytes
		})
	}
	if err := os.MkdirAll(filepath.Dir(destination), 0o700); err != nil {
		return 0, fmt.Errorf("create download directory: %w", err)
	}
	file, err := os.OpenFile(destination, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600)
	if err != nil {
		return 0, fmt.Errorf("create downloaded file: %w", err)
	}
	writer := &taildriveProgressWriter{
		writer:     file,
		transferID: transferID,
		base:       baseBytes,
		total:      totalBytes,
	}
	written, copyErr := io.Copy(writer, response.Body)
	closeErr := file.Close()
	if copyErr != nil {
		_ = os.Remove(destination)
		return written, fmt.Errorf("download Taildrive file: %w", copyErr)
	}
	if closeErr != nil {
		_ = os.Remove(destination)
		return written, fmt.Errorf("close downloaded file: %w", closeErr)
	}
	return written, nil
}

func taildriveDownloadWithProgress(profileID, deviceID, share, remotePath, destination, transferID string) (retErr error) {
	setTaildriveTransferProgress(transferID, taildriveTransferProgress{Phase: "Downloading"})
	defer releaseTaildriveTransferControl(transferID)
	defer func() {
		updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
			progress.Done = true
			progress.Paused = false
			if errors.Is(retErr, errTaildriveTransferCancelled) {
				progress.Cancelled = true
				progress.Error = ""
				progress.Phase = "Cancelled"
			} else if retErr != nil {
				progress.Error = retErr.Error()
			} else {
				if progress.BytesTotal > 0 {
					progress.BytesDone = progress.BytesTotal
				}
				progress.ItemsDone = progress.ItemsTotal
			}
		})
	}()

	normalizedPath, err := normalizeTaildriveBrowserPath(remotePath)
	if err != nil {
		return err
	}
	if normalizedPath == "" {
		return errors.New("Taildrive file path is empty")
	}
	profile, srv, client, err := taildriveActiveContext(profileID, deviceID, share)
	if err != nil {
		return err
	}
	isDir, err := taildriveRemoteIsDirectory(profile, srv, client, deviceID, share, normalizedPath)
	if err != nil {
		return err
	}
	parent := filepath.Dir(destination)
	if err := os.MkdirAll(parent, 0o700); err != nil {
		return fmt.Errorf("create download directory: %w", err)
	}
	if _, err := os.Lstat(destination); err == nil {
		return errors.New("download destination already exists")
	} else if !errors.Is(err, os.ErrNotExist) {
		return err
	}

	if !isDir {
		temp, err := os.CreateTemp(parent, ".fastexplorer-taildrive-*")
		if err != nil {
			return fmt.Errorf("create temporary download: %w", err)
		}
		tempPath := temp.Name()
		_ = temp.Close()
		_ = os.Remove(tempPath)
		totalBytes := int64(0)
		updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
			progress.ItemsTotal = 1
		})
		written, err := downloadTaildriveFileTo(profile, srv, client, deviceID, share, normalizedPath, tempPath, transferID, 0, totalBytes, true)
		if err != nil {
			_ = os.Remove(tempPath)
			return err
		}
		updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
			progress.BytesDone = written
			if progress.BytesTotal == 0 {
				progress.BytesTotal = written
			}
			progress.ItemsDone = 1
		})
		if err := waitTaildriveTransferControl(transferID); err != nil {
			_ = os.Remove(tempPath)
			return err
		}
		if err := os.Rename(tempPath, destination); err != nil {
			_ = os.Remove(tempPath)
			return fmt.Errorf("publish downloaded file: %w", err)
		}
		return nil
	}

	entries, totalBytes, sizesKnown, err := collectTaildriveDownloadTree(profile, srv, client, deviceID, share, normalizedPath)
	if err != nil {
		return err
	}
	if !sizesKnown {
		totalBytes = 0
	}
	updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
		progress.BytesTotal = totalBytes
		progress.ItemsTotal = int64(len(entries))
	})
	tempRoot, err := os.MkdirTemp(parent, ".fastexplorer-taildrive-dir-*")
	if err != nil {
		return fmt.Errorf("create temporary download folder: %w", err)
	}
	committed := false
	defer func() {
		if !committed {
			_ = os.RemoveAll(tempRoot)
		}
	}()
	var completedBytes, completedItems int64
	for _, entry := range entries {
		if err := waitTaildriveTransferControl(transferID); err != nil {
			return err
		}
		localPath := filepath.Join(tempRoot, filepath.FromSlash(entry.Relative))
		relativeCheck, err := filepath.Rel(tempRoot, localPath)
		if err != nil || relativeCheck == ".." || strings.HasPrefix(relativeCheck, ".."+string(filepath.Separator)) {
			return errors.New("Taildrive path escaped temporary download folder")
		}
		if entry.Directory {
			if err := os.MkdirAll(localPath, 0o700); err != nil {
				return err
			}
			completedItems++
		} else {
			written, err := downloadTaildriveFileTo(profile, srv, client, deviceID, share, entry.RemotePath, localPath, transferID, completedBytes, totalBytes, false)
			if err != nil {
				return err
			}
			completedBytes += written
			completedItems++
		}
		updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
			progress.BytesDone = completedBytes
			progress.ItemsDone = completedItems
		})
	}
	if err := waitTaildriveTransferControl(transferID); err != nil {
		return err
	}
	updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
		progress.Phase = "Finishing download"
	})
	if err := os.Rename(tempRoot, destination); err != nil {
		return fmt.Errorf("publish downloaded folder: %w", err)
	}
	committed = true
	return nil
}

func mergeLocalDirectoryIntoTaildrive(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, share, remoteDir, localDir, transferID string, completedBytes, completedItems *int64, totalBytes int64) error {
	remoteEntries, err := listTaildriveDirectory(profile, srv, client, deviceID, share, remoteDir)
	if err != nil {
		return fmt.Errorf("list destination folder %s: %w", remoteDir, err)
	}
	remoteByName := make(map[string]taildriveBrowserEntry, len(remoteEntries))
	for _, entry := range remoteEntries {
		remoteByName[strings.ToLower(entry.Name)] = entry
	}

	localEntries, err := os.ReadDir(localDir)
	if err != nil {
		return fmt.Errorf("read upload folder %s: %w", localDir, err)
	}
	for _, entry := range localEntries {
		if err := waitTaildriveTransferControl(transferID); err != nil {
			return err
		}
		name := entry.Name()
		if !validTaildriveLeafName(name) {
			return fmt.Errorf("invalid Taildrive filename: %s", name)
		}
		if entry.Type()&os.ModeSymlink != 0 {
			return fmt.Errorf("symbolic links are not supported: %s", filepath.Join(localDir, name))
		}
		localPath := filepath.Join(localDir, name)
		existing, exists := remoteByName[strings.ToLower(name)]
		remoteChild := pathpkg.Join(remoteDir, name)
		if exists {
			remoteChild = existing.Path
		}

		if entry.IsDir() {
			if exists && !existing.Directory {
				return fmt.Errorf("cannot merge folder %q because the destination contains a file with that name", name)
			}
			if !exists {
				if err := mkdirTaildrivePath(profile, srv, client, deviceID, share, remoteChild); err != nil {
					return err
				}
			}
			*completedItems++
			updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
				progress.BytesDone = *completedBytes
				progress.ItemsDone = *completedItems
				progress.Phase = "Merging folders"
			})
			if err := mergeLocalDirectoryIntoTaildrive(profile, srv, client, deviceID, share, remoteChild, localPath, transferID, completedBytes, completedItems, totalBytes); err != nil {
				return err
			}
			continue
		}

		if exists && existing.Directory {
			return fmt.Errorf("cannot replace destination folder %q with a file while merging folders", name)
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		if !info.Mode().IsRegular() {
			return fmt.Errorf("Taildrive upload source is not a regular file: %s", localPath)
		}
		if err := publishTaildriveFile(profile, srv, client, deviceID, share, remoteChild, localPath, transferID, *completedBytes, totalBytes, exists); err != nil {
			return err
		}
		*completedBytes += info.Size()
		*completedItems++
		updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
			progress.BytesDone = *completedBytes
			progress.ItemsDone = *completedItems
			progress.Phase = "Merging folders"
		})
	}
	return nil
}

func taildriveUpload(profileID, deviceID, share, remotePath, source string) error {
	return taildriveUploadWithProgressMode(profileID, deviceID, share, remotePath, source, "", false)
}

func taildriveUploadWithProgress(profileID, deviceID, share, remotePath, source, transferID string) error {
	return taildriveUploadWithProgressMode(profileID, deviceID, share, remotePath, source, transferID, false)
}

func taildriveUploadReplaceWithProgress(profileID, deviceID, share, remotePath, source, transferID string) error {
	return taildriveUploadWithProgressMode(profileID, deviceID, share, remotePath, source, transferID, true)
}

func taildriveUploadWithProgressMode(profileID, deviceID, share, remotePath, source, transferID string, overwrite bool) (retErr error) {
	setTaildriveTransferProgress(transferID, taildriveTransferProgress{Phase: "Uploading"})
	defer releaseTaildriveTransferControl(transferID)
	defer func() {
		updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
			progress.Done = true
			progress.Paused = false
			if errors.Is(retErr, errTaildriveTransferCancelled) {
				progress.Cancelled = true
				progress.Error = ""
				progress.Phase = "Cancelled"
			} else if retErr != nil {
				progress.Error = retErr.Error()
			} else {
				progress.BytesDone = progress.BytesTotal
				progress.ItemsDone = progress.ItemsTotal
			}
		})
	}()

	normalizedPath, err := normalizeTaildriveBrowserPath(remotePath)
	if err != nil {
		return err
	}
	if normalizedPath == "" || !validTaildriveLeafName(pathpkg.Base(normalizedPath)) {
		return errors.New("invalid Taildrive upload path")
	}
	info, err := os.Lstat(source)
	if err != nil {
		return fmt.Errorf("stat upload source: %w", err)
	}
	if info.Mode()&os.ModeSymlink != 0 {
		return errors.New("Taildrive upload does not follow symbolic links")
	}
	if !info.Mode().IsRegular() && !info.IsDir() {
		return errors.New("Taildrive upload source is not a regular file or directory")
	}

	var totalBytes, totalItems int64
	if info.IsDir() {
		err = filepath.WalkDir(source, func(localPath string, entry os.DirEntry, walkErr error) error {
			if controlErr := waitTaildriveTransferControl(transferID); controlErr != nil {
				return controlErr
			}
			if walkErr != nil {
				return walkErr
			}
			if localPath == source {
				return nil
			}
			if entry.Type()&os.ModeSymlink != 0 {
				return fmt.Errorf("symbolic links are not supported: %s", localPath)
			}
			totalItems++
			if !entry.IsDir() {
				entryInfo, infoErr := entry.Info()
				if infoErr != nil {
					return infoErr
				}
				totalBytes += entryInfo.Size()
			}
			return nil
		})
		if err != nil {
			return err
		}
	} else {
		totalBytes = info.Size()
		totalItems = 1
	}
	updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
		progress.BytesTotal = totalBytes
		progress.ItemsTotal = totalItems
	})

	profile, srv, client, err := taildriveActiveContext(profileID, deviceID, share)
	if err != nil {
		return err
	}
	if overwrite && info.IsDir() {
		targetIsDir, inspectErr := taildriveRemoteIsDirectory(profile, srv, client, deviceID, share, normalizedPath)
		if inspectErr != nil {
			return fmt.Errorf("inspect destination before folder merge: %w", inspectErr)
		}
		if targetIsDir {
			var completedBytes, completedItems int64
			updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
				progress.Phase = "Merging folders"
			})
			return mergeLocalDirectoryIntoTaildrive(
				profile,
				srv,
				client,
				deviceID,
				share,
				normalizedPath,
				source,
				transferID,
				&completedBytes,
				&completedItems,
				totalBytes,
			)
		}
	}
	temporaryPath, err := taildriveTemporarySibling(normalizedPath)
	if err != nil {
		return err
	}
	isDir := info.IsDir()
	var completedBytes, completedItems int64
	if isDir {
		if err := mkdirTaildrivePath(profile, srv, client, deviceID, share, temporaryPath); err != nil {
			return err
		}
		err = filepath.WalkDir(source, func(localPath string, entry os.DirEntry, walkErr error) error {
			if controlErr := waitTaildriveTransferControl(transferID); controlErr != nil {
				return controlErr
			}
			if walkErr != nil {
				return walkErr
			}
			if localPath == source {
				return nil
			}
			if entry.Type()&os.ModeSymlink != 0 {
				return fmt.Errorf("symbolic links are not supported: %s", localPath)
			}
			relative, relErr := filepath.Rel(source, localPath)
			if relErr != nil {
				return relErr
			}
			parts := strings.Split(filepath.ToSlash(relative), "/")
			remoteChild := temporaryPath
			for _, part := range parts {
				if !validTaildriveLeafName(part) {
					return fmt.Errorf("invalid Taildrive filename: %s", part)
				}
				remoteChild = pathpkg.Join(remoteChild, part)
			}
			if entry.IsDir() {
				err := mkdirTaildrivePath(profile, srv, client, deviceID, share, remoteChild)
				if err == nil {
					completedItems++
				}
				updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
					progress.ItemsDone = completedItems
				})
				return err
			}
			entryInfo, infoErr := entry.Info()
			if infoErr != nil {
				return infoErr
			}
			if err := putTaildriveFile(profile, srv, client, deviceID, share, remoteChild, localPath, transferID, completedBytes, totalBytes); err != nil {
				return err
			}
			completedBytes += entryInfo.Size()
			completedItems++
			updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
				progress.BytesDone = completedBytes
				progress.ItemsDone = completedItems
			})
			return nil
		})
	} else {
		err = putTaildriveFile(profile, srv, client, deviceID, share, temporaryPath, source, transferID, 0, totalBytes)
		if err == nil {
			completedBytes = totalBytes
			completedItems = 1
		}
	}
	if err != nil {
		cleanupTaildriveTemporary(profile, srv, client, deviceID, share, temporaryPath, isDir)
		return err
	}
	if err := waitTaildriveTransferControl(transferID); err != nil {
		cleanupTaildriveTemporary(profile, srv, client, deviceID, share, temporaryPath, isDir)
		return err
	}
	updateTaildriveTransfer(transferID, func(progress *taildriveTransferProgress) {
		progress.Phase = "Finishing upload"
		progress.BytesDone = completedBytes
		progress.ItemsDone = completedItems
	})
	response, err := doTaildriveMoveRequest(
		profile,
		srv,
		client,
		deviceID,
		taildriveRemotePath(share, temporaryPath, isDir),
		taildriveRemotePath(share, normalizedPath, isDir),
		overwrite,
	)
	if err != nil {
		cleanupTaildriveTemporary(profile, srv, client, deviceID, share, temporaryPath, isDir)
		return err
	}
	if err := finishTaildriveMutation(response, "Taildrive publish upload"); err != nil {
		cleanupTaildriveTemporary(profile, srv, client, deviceID, share, temporaryPath, isDir)
		return err
	}
	return nil
}

func taildriveMkdir(profileID, deviceID, share, remotePath string) error {
	normalizedPath, err := normalizeTaildriveBrowserPath(remotePath)
	if err != nil {
		return err
	}
	if normalizedPath == "" || !validTaildriveLeafName(pathpkg.Base(normalizedPath)) {
		return errors.New("invalid Taildrive folder path")
	}
	profile, srv, client, err := taildriveActiveContext(profileID, deviceID, share)
	if err != nil {
		return err
	}
	response, err := doTaildriveRequest(profile, srv, client, deviceID, "MKCOL", taildriveRemotePath(share, normalizedPath, true), nil, nil)
	if err != nil {
		return err
	}
	return finishTaildriveMutation(response, "Taildrive create folder")
}

func taildriveDelete(profileID, deviceID, share, remotePath string) error {
	normalizedPath, err := normalizeTaildriveBrowserPath(remotePath)
	if err != nil {
		return err
	}
	if normalizedPath == "" {
		return errors.New("refusing to delete a Taildrive share root")
	}
	profile, srv, client, err := taildriveActiveContext(profileID, deviceID, share)
	if err != nil {
		return err
	}
	isDir, err := taildriveRemoteIsDirectory(profile, srv, client, deviceID, share, normalizedPath)
	if err != nil {
		return err
	}
	response, err := doTaildriveRequest(profile, srv, client, deviceID, http.MethodDelete, taildriveRemotePath(share, normalizedPath, isDir), nil, nil)
	if err != nil {
		return err
	}
	return finishTaildriveMutation(response, "Taildrive delete")
}

func taildriveRename(profileID, deviceID, share, remotePath, newName string) error {
	normalizedPath, err := normalizeTaildriveBrowserPath(remotePath)
	if err != nil {
		return err
	}
	newName = strings.TrimSpace(newName)
	if normalizedPath == "" || !validTaildriveLeafName(newName) {
		return errors.New("invalid Taildrive rename target")
	}
	profile, srv, client, err := taildriveActiveContext(profileID, deviceID, share)
	if err != nil {
		return err
	}
	isDir, err := taildriveRemoteIsDirectory(profile, srv, client, deviceID, share, normalizedPath)
	if err != nil {
		return err
	}
	parent := pathpkg.Dir(normalizedPath)
	if parent == "." {
		parent = ""
	}
	destinationPath := pathpkg.Join(parent, newName)
	if destinationPath == normalizedPath {
		return nil
	}
	response, err := doTaildriveMoveRequest(
		profile,
		srv,
		client,
		deviceID,
		taildriveRemotePath(share, normalizedPath, isDir),
		taildriveRemotePath(share, destinationPath, isDir),
		false,
	)
	if err != nil {
		return err
	}
	return finishTaildriveMutation(response, "Taildrive rename")
}

func streamTaildriveFile(profile *profileBridge, srv *tsnet.Server, client *local.Client, deviceID, share, remotePath string, w http.ResponseWriter, r *http.Request) {
	headers := make(http.Header)
	if rangeHeader := r.Header.Get("Range"); rangeHeader != "" {
		headers.Set("Range", rangeHeader)
	}
	response, err := doTaildriveRequest(profile, srv, client, deviceID, http.MethodGet, taildriveRemotePath(share, remotePath, false), nil, headers)
	if err != nil {
		http.Error(w, "Taildrive download failed: "+err.Error(), http.StatusBadGateway)
		return
	}
	defer response.Body.Close()
	for _, key := range []string{"Content-Type", "Content-Length", "Content-Range", "ETag", "Last-Modified", "Accept-Ranges"} {
		if value := response.Header.Get(key); value != "" {
			w.Header().Set(key, value)
		}
	}
	if w.Header().Get("Content-Type") == "" {
		w.Header().Set("Content-Type", "application/octet-stream")
	}
	w.WriteHeader(response.StatusCode)
	_, _ = io.Copy(w, response.Body)
}

func spoolTaildriveUpload(profile *profileBridge, source io.Reader) (*os.File, string, error) {
	profile.mu.RLock()
	stateDir := profile.stateDir
	profile.mu.RUnlock()
	if stateDir == "" {
		return nil, "", errors.New("Taildrive profile state directory is unavailable")
	}
	dir := filepath.Join(stateDir, "taildrive-upload")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, "", fmt.Errorf("create Taildrive upload spool: %w", err)
	}
	file, err := os.CreateTemp(dir, "upload-*")
	if err != nil {
		return nil, "", fmt.Errorf("create Taildrive upload spool file: %w", err)
	}
	path := file.Name()
	cleanup := func(err error) (*os.File, string, error) {
		_ = file.Close()
		_ = os.Remove(path)
		return nil, "", err
	}
	if err := file.Chmod(0o600); err != nil {
		return cleanup(fmt.Errorf("secure Taildrive upload spool file: %w", err))
	}
	if _, err := io.Copy(file, source); err != nil {
		return cleanup(fmt.Errorf("spool Taildrive upload: %w", err))
	}
	if _, err := file.Seek(0, io.SeekStart); err != nil {
		return cleanup(fmt.Errorf("rewind Taildrive upload spool: %w", err))
	}
	return file, path, nil
}

func handleTaildriveBrowserPost(profile *profileBridge, srv *tsnet.Server, client *local.Client, prefix, deviceID, share, remotePath string, w http.ResponseWriter, r *http.Request) {
	action := r.URL.Query().Get("action")
	switch action {
	case "upload":
		reader, err := r.MultipartReader()
		if err != nil {
			http.Error(w, "invalid upload", http.StatusBadRequest)
			return
		}
		var uploaded bool
		for {
			part, err := reader.NextPart()
			if errors.Is(err, io.EOF) {
				break
			}
			if err != nil {
				http.Error(w, "invalid upload", http.StatusBadRequest)
				return
			}
			name := pathpkg.Base(strings.TrimSpace(part.FileName()))
			if !validTaildriveLeafName(name) {
				_ = part.Close()
				http.Error(w, "invalid upload filename", http.StatusBadRequest)
				return
			}
			target := pathpkg.Join(remotePath, name)
			headers := make(http.Header)
			if contentType := part.Header.Get("Content-Type"); contentType != "" {
				headers.Set("Content-Type", contentType)
			}
			spool, spoolPath, err := spoolTaildriveUpload(profile, part)
			_ = part.Close()
			if err != nil {
				http.Error(w, "upload failed: "+err.Error(), http.StatusBadGateway)
				return
			}
			response, err := doTaildriveRequest(profile, srv, client, deviceID, http.MethodPut, taildriveRemotePath(share, target, false), taildriveRewindableReader{ReadSeeker: spool}, headers)
			_ = spool.Close()
			_ = os.Remove(spoolPath)
			if err != nil {
				http.Error(w, "upload failed: "+err.Error(), http.StatusBadGateway)
				return
			}
			_, _ = io.Copy(io.Discard, response.Body)
			_ = response.Body.Close()
			if response.StatusCode < 200 || response.StatusCode >= 300 {
				http.Error(w, "upload rejected: "+response.Status, http.StatusBadGateway)
				return
			}
			uploaded = true
		}
		if !uploaded {
			http.Error(w, "no file selected", http.StatusBadRequest)
			return
		}
	case "mkdir":
		if err := r.ParseForm(); err != nil {
			http.Error(w, "invalid folder name", http.StatusBadRequest)
			return
		}
		name := strings.TrimSpace(r.Form.Get("name"))
		if !validTaildriveLeafName(name) {
			http.Error(w, "invalid folder name", http.StatusBadRequest)
			return
		}
		target := pathpkg.Join(remotePath, name)
		response, err := doTaildriveRequest(profile, srv, client, deviceID, "MKCOL", taildriveRemotePath(share, target, true), nil, nil)
		if err != nil {
			http.Error(w, "create folder failed: "+err.Error(), http.StatusBadGateway)
			return
		}
		_, _ = io.Copy(io.Discard, response.Body)
		_ = response.Body.Close()
		if response.StatusCode < 200 || response.StatusCode >= 300 {
			http.Error(w, "create folder rejected: "+response.Status, http.StatusBadGateway)
			return
		}
	default:
		http.Error(w, "unknown action", http.StatusBadRequest)
		return
	}
	location := taildriveBrowserURL(prefix, deviceID, share, remotePath)
	http.Redirect(w, r, location, http.StatusSeeOther)
}

func taildriveBrowserURL(prefix, deviceID, share, remotePath string) string {
	values := url.Values{}
	if deviceID != "" {
		values.Set("device", deviceID)
	}
	if share != "" {
		values.Set("share", share)
	}
	if remotePath != "" {
		values.Set("path", remotePath)
	}
	if encoded := values.Encode(); encoded != "" {
		return prefix + "?" + encoded
	}
	return prefix
}

func taildriveBrowserActionURL(prefix, deviceID, share, remotePath, action, csrfToken string) string {
	values := url.Values{}
	values.Set("device", deviceID)
	values.Set("share", share)
	if remotePath != "" {
		values.Set("path", remotePath)
	}
	values.Set("action", action)
	values.Set("csrf", csrfToken)
	return prefix + "?" + values.Encode()
}

func renderTaildriveGatewayIndex(w http.ResponseWriter, prefix string, devices []taildriveDeviceInfo) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	_, _ = io.WriteString(w, taildriveHTMLHeader("FastExplorer Taildrive"))
	_, _ = io.WriteString(w, "<h1>Taildrive</h1><p>Shares reachable through this FastExplorer Tailnet profile.</p>")
	count := 0
	for _, device := range devices {
		if !device.Online || len(device.Shares) == 0 {
			continue
		}
		name := device.HostName
		if name == "" {
			name = device.Target
		}
		_, _ = fmt.Fprintf(w, "<section><h2>%s</h2><ul>", html.EscapeString(name))
		for _, share := range device.Shares {
			href := taildriveBrowserURL(prefix, device.ID, share, "")
			_, _ = fmt.Fprintf(w, "<li><a href=\"%s\">%s</a></li>", html.EscapeString(href), html.EscapeString(share))
			count++
		}
		_, _ = io.WriteString(w, "</ul></section>")
	}
	if count == 0 {
		_, _ = io.WriteString(w, "<p>No Taildrive shares are currently available. Return to FastExplorer and refresh the Tailnet profile.</p>")
	}
	_, _ = io.WriteString(w, taildriveHTMLFooter())
}

func renderTaildriveDirectory(w http.ResponseWriter, prefix, csrfToken, deviceID, share, remotePath string, entries []taildriveBrowserEntry) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	title := share
	if remotePath != "" {
		title += " / " + remotePath
	}
	_, _ = io.WriteString(w, taildriveHTMLHeader(title))
	_, _ = fmt.Fprintf(w, "<nav><a href=\"%s\">All shares</a>", html.EscapeString(prefix))
	if remotePath != "" {
		parent := pathpkg.Dir(remotePath)
		if parent == "." {
			parent = ""
		}
		_, _ = fmt.Fprintf(w, " · <a href=\"%s\">Up</a>", html.EscapeString(taildriveBrowserURL(prefix, deviceID, share, parent)))
	}
	_, _ = io.WriteString(w, "</nav>")
	_, _ = fmt.Fprintf(w, "<h1>%s</h1><table><thead><tr><th>Name</th><th>Size</th><th>Modified</th></tr></thead><tbody>", html.EscapeString(title))
	for _, entry := range entries {
		href := taildriveBrowserURL(prefix, deviceID, share, entry.Path)
		name := entry.Name
		if entry.Directory {
			name += "/"
		}
		size := entry.Size
		if entry.Directory {
			size = "—"
		} else if n, err := strconv.ParseInt(size, 10, 64); err == nil {
			size = humanTaildriveBytes(n)
		}
		_, _ = fmt.Fprintf(w, "<tr><td><a href=\"%s\">%s</a></td><td>%s</td><td>%s</td></tr>", html.EscapeString(href), html.EscapeString(name), html.EscapeString(size), html.EscapeString(entry.Modified))
	}
	_, _ = io.WriteString(w, "</tbody></table>")
	uploadAction := taildriveBrowserActionURL(prefix, deviceID, share, remotePath, "upload", csrfToken)
	mkdirAction := taildriveBrowserActionURL(prefix, deviceID, share, remotePath, "mkdir", csrfToken)
	_, _ = fmt.Fprintf(w, `<section class="tools"><h2>Upload</h2><form method="post" enctype="multipart/form-data" action="%s"><input type="file" name="file" required><button type="submit">Upload</button></form>`, html.EscapeString(uploadAction))
	_, _ = fmt.Fprintf(w, `<h2>New folder</h2><form method="post" action="%s"><input name="name" required autocomplete="off"><button type="submit">Create</button></form></section>`, html.EscapeString(mkdirAction))
	_, _ = io.WriteString(w, taildriveHTMLFooter())
}

func humanTaildriveBytes(value int64) string {
	const unit = 1024
	if value < unit {
		return fmt.Sprintf("%d B", value)
	}
	div, exp := int64(unit), 0
	for n := value / unit; n >= unit && exp < 3; n /= unit {
		div *= unit
		exp++
	}
	return fmt.Sprintf("%.1f %ciB", float64(value)/float64(div), "KMGT"[exp])
}

func taildriveHTMLHeader(title string) string {
	return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>` + html.EscapeString(title) + `</title><style>body{font:16px system-ui,sans-serif;max-width:900px;margin:auto;padding:20px;color:#171717;background:#fafafa}a{color:#075db5;text-decoration:none}a:hover{text-decoration:underline}table{width:100%;border-collapse:collapse}th,td{text-align:left;padding:10px 8px;border-bottom:1px solid #ddd}th{font-size:.85rem;color:#666}.tools{margin-top:28px;padding:16px;border:1px solid #ddd;border-radius:10px;background:#fff}form{display:flex;gap:8px;flex-wrap:wrap;margin-bottom:12px}input,button{font:inherit;padding:9px 10px}button{cursor:pointer}nav{margin-bottom:16px}@media(prefers-color-scheme:dark){body{color:#eee;background:#151515}a{color:#74b9ff}th{color:#aaa}th,td{border-color:#333}.tools{background:#1d1d1d;border-color:#3a3a3a}input,button{color:#eee;background:#242424;border:1px solid #555}}</style></head><body>`
}
func taildriveHTMLFooter() string {
	return `<footer><p>Served locally by FastExplorer over the embedded Tailnet connection.</p></footer></body></html>`
}
