//go:build !linux

package daemon

import (
	"fmt"
	"net"
)

func adoptSystemdTCP(string) (net.Listener, error) {
	return nil, fmt.Errorf("systemd TCP activation requires Linux")
}
