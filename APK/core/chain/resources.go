package chain

const RuleVersion = "meta-rules-dat-19f81c0fda5b505c0b186bcbc67cc5f8226aafd3"

type RuleFile struct {
	Name   string `json:"name"`
	SHA256 string `json:"sha256"`
	Bytes  int64  `json:"bytes"`
}

func RuleFiles() []RuleFile {
	return []RuleFile{
		{Name: "geoip.dat", SHA256: "d4ec4c5e5d5a38c1399964ea9fd8c94288426a3b31b483904b67819fc07f4466", Bytes: 16503149},
		{Name: "geosite.dat", SHA256: "55c0ff0bb7a75de661de7a30984ece344d9c7840a37a92893b59af9f4baed280", Bytes: 4349121},
	}
}
