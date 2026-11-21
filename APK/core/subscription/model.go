package subscription

const CapabilityVersion = 1

type Node struct {
	ID        string         `json:"id"`
	Name      string         `json:"name"`
	Protocol  string         `json:"protocol"`
	Server    string         `json:"server"`
	Port      int            `json:"port"`
	Options   map[string]any `json:"options"`
	Rejection string         `json:"rejection"`
}

type Result struct {
	CapabilityVersion int    `json:"capabilityVersion"`
	Nodes             []Node `json:"nodes"`
	Duplicates        int    `json:"duplicates"`
}
