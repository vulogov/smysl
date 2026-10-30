// Command xi is the Go implementation's half of SMYSL-2.7 §5.4's cross-implementation harness.
//
//	go run ./cmd/xi verify DIR   one JSON line; exit 0 when the collection verifies
//	go run ./cmd/xi write DIR    what this implementation produces, for the Rust to import
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	smysl "github.com/vulogov/smysl/go"
)

func main() {
	if len(os.Args) != 3 {
		fmt.Fprintln(os.Stderr, "usage: xi verify DIR | write DIR")
		os.Exit(2)
	}
	switch os.Args[1] {
	case "verify":
		r, err := smysl.VerifyExport(os.Args[2])
		if err != nil {
			fmt.Println(`{"ok":false,"problems":[` + fmt.Sprintf("%q", err.Error()) + `]}`)
			os.Exit(1)
		}
		out, _ := json.Marshal(r)
		fmt.Println(string(out))
		if !r.Ok {
			os.Exit(1)
		}
	case "write":
		_, here, _, _ := runtime.Caller(0)
		wire := filepath.Join(filepath.Dir(here), "..", "..", "..", "fixtures", "wire")
		names, err := smysl.WriteProduced(wire, os.Args[2])
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		for _, n := range names {
			fmt.Println(n)
		}
	default:
		fmt.Fprintln(os.Stderr, "usage: xi verify DIR | write DIR")
		os.Exit(2)
	}
}
