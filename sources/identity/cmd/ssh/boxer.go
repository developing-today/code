package ssh

import (
	"fmt"
	"time"

	"github.com/charmbracelet/bubbles/spinner"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/log"
	boxer "github.com/treilik/bubbleboxer"
)

const (
	upperAddr       = "upper"
	leftAddr        = "left"
	middleAddr      = "middle"
	rightAddr       = "right"
	lowerAddr       = "lower"
	chat            = "Chat"
	upload          = "Upload"
	game            = "Game"
	defaultEmpty    = " defaultEmpty"
	defaultDream    = " defaultDream"
	defaultHelp     = " defaultHelp"
	defaultChat     = " defaultChat"
	defaultViewport = " defaultViewport"
	defaultUpload   = " defaultUpload"
)

func (o *model) defaultModels(modelName string) tea.Model {
	log.Info("Creating default model", "model", modelName)
	switch modelName {
	case chat:
		return o.chatModel()
	case upload:
		return stringer("Upload")
	case game:
		return stringer("Game")
	case defaultChat:
		return stringer("chat")
	case defaultEmpty:
		return stringer("")
	case defaultDream:
		return stringer("tel'aran'rhiod, the world of dreams")
	case defaultViewport:
		v := viewport.New(0, 0)
		v.SetContent("el dorado")
		return viewPortHolder{v}
	case defaultHelp:
		return stringer(fmt.Sprintf("%s: use ctrl+c to quit", "boxer"))
	case defaultUpload:
		return stringer("upload")
	default:
		return stringer("unknown")
	}
}

func arrayContains(arr []string, s string) bool {
	for _, a := range arr {
		if a == s {
			return true
		}
	}
	return false
}

func (o *model) NewBoxerModel(msg tea.Msg) (m BoxerModel, cmd tea.Cmd) {
	selected := GetSelected(o)
	m = BoxerModel{tui: boxer.Boxer{}, o: o}
	m.o.boxer = &m
	columns := []boxer.Node{}

	leftModelName := o.layout[leftAddr]
	if leftModelName == "" {
		log.Info("No model name found for left column, using default", "model", defaultChat)
		leftModelName = defaultChat
	}
	var leftModel tea.Model
	if arrayContains(selected, leftModelName) {
		leftModel = o.models[leftModelName]
		if leftModel == nil {
			leftModel = o.defaultModels(leftModelName)
			o.models[leftModelName] = leftModel
		}
	} else {
		leftModel = nil
	}
	if leftModel != nil {
		columns = append(columns, stripErr(m.tui.CreateLeaf(leftAddr, leftModel)))
	}

	middleModelName := o.layout[middleAddr]
	if middleModelName == "" {
		log.Info("No model name found for middle column, using default", "model", defaultViewport)
		middleModelName = defaultViewport
	}
	var middleModel tea.Model
	if arrayContains(selected, middleModelName) {
		middleModel = o.models[middleModelName]
		if middleModel == nil {
			middleModel = o.defaultModels(middleModelName)
			o.models[middleModelName] = middleModel
		}
	} else {
		middleModel = nil
	}
	if middleModel != nil {
		columns = append(columns, stripErr(m.tui.CreateLeaf(middleAddr, middleModel)))
	}

	rightModelName := o.layout[rightAddr]
	if rightModelName == "" {
		log.Info("No model name found for right column, using default", "model", defaultUpload)
		rightModelName = defaultUpload
	}
	var rightModel tea.Model
	if arrayContains(selected, rightModelName) {
		rightModel = o.models[rightModelName]
		if rightModel == nil {
			rightModel = o.defaultModels(rightModelName)
			o.models[rightModelName] = rightModel
		}
	} else {
		rightModel = nil
	}
	if rightModel != nil {
		columns = append(columns, stripErr(m.tui.CreateLeaf(rightAddr, rightModel)))
	}
	if len(columns) == 0 {
		log.Info("No columns found, using default", "model", defaultEmpty)
		if m.o.models[defaultEmpty] == nil {
			m.o.models[defaultEmpty] = o.defaultModels(defaultEmpty)
		}
		columns = append(columns, stripErr(m.tui.CreateLeaf(leftAddr, m.o.models[defaultEmpty])))
	}
	if m.o.models[defaultDream] == nil {
		m.o.models[defaultDream] = o.defaultModels(defaultDream)
	}
	if m.o.models[defaultHelp] == nil {
		m.o.models[defaultHelp] = o.defaultModels(defaultHelp)
	}
	m.tui.LayoutTree = boxer.Node{
		VerticalStacked: true,
		SizeFunc: func(_ boxer.Node, widthOrHeight int) []int {
			return []int{
				// since this node is vertical stacked return the height partioning since the width stays for all children fixed
				1,
				widthOrHeight - 2,
				1,
				// make also sure that the amount of the returned ints match the amount of children:
				// in this case two, but in more complex cases read the amount of the chilren from the len(boxer.Node.Children)
			}
		},
		Children: []boxer.Node{
			stripErr(m.tui.CreateLeaf(upperAddr, m.o.models[defaultDream])),
			{
				Children: columns,
			},
			stripErr(m.tui.CreateLeaf(lowerAddr, m.o.models[defaultHelp])),
		},
	}
	m.tui.UpdateSize(tea.WindowSizeMsg{Width: o.width, Height: o.height})
	m.Update(msg)
	return m, nil
}

func stripErr(n boxer.Node, _ error) boxer.Node {
	return n
}

type BoxerModel struct {
	tui boxer.Boxer
	o   *model
}

func (m BoxerModel) Init() tea.Cmd {
	return spinner.Tick
}

func (m BoxerModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.String() {
		case "q", "ctrl+c":
			return m, tea.Quit
		case "tab", "#":
			m.o.mode = loadingMode
			m.o.loadInitDateTime = time.Now()
			return m.o, m.o.spinner.Tick
		}
	case tea.WindowSizeMsg:
		m.tui.UpdateSize(msg)
	case spinner.TickMsg:
		var cmd tea.Cmd
		m.editModel("spinner", func(v tea.Model) (tea.Model, error) {
			v, cmd = v.Update(msg)
			return v, nil
		})
		return m, cmd
	}
	return m, nil
}

func (m BoxerModel) View() string {
	return m.tui.View()
}

func (m *BoxerModel) editModel(addr string, edit func(tea.Model) (tea.Model, error)) error {
	if edit == nil {
		return fmt.Errorf("no edit function provided")
	}
	v, ok := m.tui.ModelMap[addr]
	if !ok {
		return fmt.Errorf("no model with address '%s' found", addr)
	}
	v, err := edit(v)
	if err != nil {
		return err
	}
	m.tui.ModelMap[addr] = v
	return nil
}

type stringer string

func (s stringer) String() string {
	return string(s)
}

// satisfy the tea.Model interface
func (s stringer) Init() tea.Cmd { return nil }

func (s stringer) Update(msg tea.Msg) (tea.Model, tea.Cmd) { return s, nil }

func (s stringer) View() string { return s.String() }

type spinnerHolder struct {
	m spinner.Model
}

func (s spinnerHolder) Init() tea.Cmd {
	return s.m.Tick
}

func (s spinnerHolder) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	m, cmd := s.m.Update(msg)
	s.m = m
	return s, cmd
}

func (s spinnerHolder) View() string {
	return s.m.View()
}

type viewPortHolder struct {
	m viewport.Model
}

func (v viewPortHolder) Init() tea.Cmd {
	return nil
}

func (v viewPortHolder) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	size, ok := msg.(tea.WindowSizeMsg)
	if ok {
		v.m.Width = size.Width
		v.m.Height = size.Height
		return v, nil
	}
	m, cmd := v.m.Update(msg)
	v.m = m
	return v, cmd
}

func (v viewPortHolder) View() string {
	return v.m.View()
}
