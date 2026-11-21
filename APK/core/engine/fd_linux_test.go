//go:build linux && with_gvisor

package engine

import (
	"os"
	"testing"

	"github.com/metacubex/gvisor/pkg/tcpip/link/fdbased"
	"github.com/metacubex/gvisor/pkg/tcpip/stack"
	tun "github.com/metacubex/sing-tun"
	"golang.org/x/sys/unix"
)

func TestGVisorFiftyEndpointLifetimesDoNotLeakFDs(t *testing.T) {
	count := func() int {
		entries, err := os.ReadDir("/proc/self/fd")
		if err != nil {
			t.Fatal(err)
		}
		return len(entries)
	}
	baseline := count()
	for i := 0; i < 50; i++ {
		for _, attach := range []bool{false, true} {
			fds := make([]int, 2)
			if err := unix.Pipe2(fds, unix.O_CLOEXEC|unix.O_NONBLOCK); err != nil {
				t.Fatal(err)
			}
			endpoint, err := fdbased.New(&fdbased.Options{FDs: []int{fds[0]}, MTU: 1500, ProcessorsPerChannel: 1})
			if err != nil {
				unix.Close(fds[0])
				unix.Close(fds[1])
				t.Fatal(err)
			}
			if attach {
				ipStack, err := tun.NewGVisorStackWithOptions(endpoint, stack.NICOptions{})
				if err != nil {
					endpoint.Close()
					unix.Close(fds[0])
					unix.Close(fds[1])
					t.Fatal(err)
				}
				endpoint.Attach(nil)
				ipStack.Close()
			}
			endpoint.Close()
			endpoint.Close()
			unix.Close(fds[0])
			unix.Close(fds[1])
			if current := count(); current != baseline {
				t.Fatalf("cycle %d attached=%v fd count=%d baseline=%d", i, attach, current, baseline)
			}
		}
	}
}
