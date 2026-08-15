package main

import (
	"context"
	"io/fs"
	"os"
	pathpkg "path"
	"path/filepath"
	"strings"

	"golang.org/x/net/webdav"
)

// rootedWebDAVFS uses os.Root so a symlink inside the share cannot escape
// the configured WebDAV tree. webdav.Dir does not provide that guarantee.
type rootedWebDAVFS struct {
	root *os.Root
}

func newRootedWebDAVFS(root *os.Root) *rootedWebDAVFS {
	return &rootedWebDAVFS{root: root}
}

func webDAVRootName(name string) (string, error) {
	if strings.ContainsRune(name, '\x00') || strings.ContainsRune(name, '\\') {
		return "", os.ErrInvalid
	}
	name = strings.TrimLeft(name, "/")
	if name == "" {
		return ".", nil
	}
	for _, component := range strings.Split(name, "/") {
		if component == ".." {
			return "", os.ErrPermission
		}
	}
	name = pathpkg.Clean(name)
	if name == "." {
		return ".", nil
	}
	if !fs.ValidPath(name) {
		return "", os.ErrInvalid
	}
	return filepath.FromSlash(name), nil
}

func (r *rootedWebDAVFS) Mkdir(_ context.Context, name string, perm os.FileMode) error {
	name, err := webDAVRootName(name)
	if err != nil {
		return err
	}
	return r.root.Mkdir(name, perm)
}

func (r *rootedWebDAVFS) OpenFile(
	_ context.Context,
	name string,
	flag int,
	perm os.FileMode,
) (webdav.File, error) {
	name, err := webDAVRootName(name)
	if err != nil {
		return nil, err
	}
	return r.root.OpenFile(name, flag, perm)
}

func (r *rootedWebDAVFS) RemoveAll(_ context.Context, name string) error {
	name, err := webDAVRootName(name)
	if err != nil {
		return err
	}
	if name == "." {
		return os.ErrInvalid
	}
	return r.root.RemoveAll(name)
}

func (r *rootedWebDAVFS) Rename(_ context.Context, oldName, newName string) error {
	oldName, err := webDAVRootName(oldName)
	if err != nil {
		return err
	}
	newName, err = webDAVRootName(newName)
	if err != nil {
		return err
	}
	if oldName == "." || newName == "." {
		return os.ErrInvalid
	}
	return r.root.Rename(oldName, newName)
}

func (r *rootedWebDAVFS) Stat(_ context.Context, name string) (os.FileInfo, error) {
	name, err := webDAVRootName(name)
	if err != nil {
		return nil, err
	}
	return r.root.Stat(name)
}
