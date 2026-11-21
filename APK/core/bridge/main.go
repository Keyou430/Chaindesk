//go:build android && cgo

package main

/*
#include <stdlib.h>
#include <stdint.h>
typedef int (*protect_callback)(int);
static int invoke_protect(uintptr_t callback, int fd) {
    return ((protect_callback)callback)(fd);
}
*/
import "C"

import (
	"encoding/json"
	"time"
	"unsafe"

	"github.com/Keyou430/Chaindesk/APK/core/chain"
	"github.com/Keyou430/Chaindesk/APK/core/engine"
	"github.com/Keyou430/Chaindesk/APK/core/subscription"
)

//export ChainDeskSelfCheck
func ChainDeskSelfCheck() *C.char {
	encoded, _ := json.Marshal(engine.NativeSelfCheck())
	return C.CString(string(encoded))
}

//export ChainDeskValidateFixture
func ChainDeskValidateFixture(data *C.char, length C.int) C.int {
	if length <= 0 || length > 16*1024 {
		return 0
	}
	if engine.ValidateFixture(C.GoBytes(unsafe.Pointer(data), length)) != nil {
		return 0
	}
	return 1
}

//export ChainDeskStart
func ChainDeskStart(data *C.char, length C.int, fd C.int, callback C.uintptr_t) C.ulonglong {
	var protect func(int) bool
	if callback != 0 {
		protect = func(socket int) bool { return C.invoke_protect(callback, C.int(socket)) != 0 }
	}
	id, err := engine.StartSession(C.GoBytes(unsafe.Pointer(data), length), int(fd), protect)
	if err != nil {
		return 0
	}
	return C.ulonglong(id)
}

//export ChainDeskStop
func ChainDeskStop(id C.ulonglong) C.int {
	if engine.StopSession(uint64(id)) != nil {
		return 0
	}
	return 1
}

//export ChainDeskBlockSockets
func ChainDeskBlockSockets(id C.ulonglong) { engine.BlockSockets(uint64(id)) }

//export ChainDeskCancelProbe
func ChainDeskCancelProbe(id C.ulonglong) { engine.CancelProbe(uint64(id)) }

//export ChainDeskActivate
func ChainDeskActivate(id C.ulonglong) C.int {
	if engine.ActivateSession(uint64(id)) {
		return 1
	}
	return 0
}

func jsonResult(value any, err error) *C.char {
	if err != nil {
		value = map[string]string{"errorCode": err.Error()}
	}
	encoded, encodeErr := json.Marshal(value)
	if encodeErr != nil {
		encoded = []byte(`{"errorCode":"CORE_ENCODING_FAILED"}`)
	}
	return C.CString(string(encoded))
}

//export ChainDeskImportSubscription
func ChainDeskImportSubscription(data *C.char, length C.int) *C.char {
	result, err := subscription.Parse(C.GoBytes(unsafe.Pointer(data), length))
	return jsonResult(result, err)
}

//export ChainDeskCompileChain
func ChainDeskCompileChain(data *C.char, length C.int) *C.char {
	result, err := engine.CompileChain(C.GoBytes(unsafe.Pointer(data), length))
	return jsonResult(result, err)
}

//export ChainDeskStartChain
func ChainDeskStartChain(data *C.char, length C.int, fd C.int, callback C.uintptr_t) C.ulonglong {
	var protect func(int) bool
	if callback != 0 {
		protect = func(socket int) bool { return C.invoke_protect(callback, C.int(socket)) != 0 }
	}
	id, err := engine.StartChain(C.GoBytes(unsafe.Pointer(data), length), int(fd), protect)
	if err != nil {
		return 0
	}
	return C.ulonglong(id)
}

//export ChainDeskProbe
func ChainDeskProbe(id C.ulonglong, data *C.char, length C.int) *C.char {
	var input struct {
		Path         string `json:"path"`
		ExpectedIPv4 string `json:"expectedIpv4"`
		BudgetMillis int    `json:"budgetMillis"`
	}
	if json.Unmarshal(C.GoBytes(unsafe.Pointer(data), length), &input) != nil || input.BudgetMillis < 1 || input.BudgetMillis > 30000 {
		return jsonResult(map[string]string{"errorCode": "PROBE_INPUT_INVALID"}, nil)
	}
	return jsonResult(engine.ProbeSession(uint64(id), input.Path, input.ExpectedIPv4, time.Duration(min(input.BudgetMillis, 30000))*time.Millisecond), nil)
}

//export ChainDeskRuleManifest
func ChainDeskRuleManifest() *C.char {
	return jsonResult(map[string]any{"ruleVersion": chain.RuleVersion, "files": chain.RuleFiles()}, nil)
}

//export ChainDeskSetRulesDirectory
func ChainDeskSetRulesDirectory(data *C.char, length C.int) *C.char {
	var input struct {
		Path string `json:"path"`
	}
	if json.Unmarshal(C.GoBytes(unsafe.Pointer(data), length), &input) != nil {
		return jsonResult(map[string]string{"errorCode": "RULE_RESOURCE_UNAVAILABLE"}, nil)
	}
	err := engine.SetRulesDirectory(input.Path)
	return jsonResult(map[string]bool{"success": err == nil}, err)
}

//export ChainDeskFree
func ChainDeskFree(value *C.char) {
	C.free(unsafe.Pointer(value))
}

func main() {}
