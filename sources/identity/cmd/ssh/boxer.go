package ssh

import (
	"fmt"
	"time"

	"github.com/charmbracelet/bubbles/spinner"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	boxer "github.com/treilik/bubbleboxer"
)

const (
	upperAddr  = "upper"
	leftAddr   = "left"
	middleAddr = "middle"
	rightAddr  = "right"
	lowerAddr  = "lower"
)

func NewBoxerModel(o model, msg tea.Msg) (BoxerModel, tea.Cmd) {
	m := BoxerModel{tui: boxer.Boxer{}, o: o}
	columns := []boxer.Node{}

	columns = append(columns, stripErr(m.tui.CreateLeaf(leftAddr, stringer("chat"))))

	v := viewport.New(0, 0)
	v.SetContent("el dorado")
	columns = append(columns, stripErr(m.tui.CreateLeaf(middleAddr, viewPortHolder{v})))

	columns = append(columns, stripErr(m.tui.CreateLeaf(rightAddr, stringer("upload"))))
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
			stripErr(m.tui.CreateLeaf(upperAddr, stringer("tel'aran'rhiod, the world of dreams"))),
			{
				Children: columns,
			},
			stripErr(m.tui.CreateLeaf(lowerAddr, stringer(fmt.Sprintf("%s: use ctrl+c to quit", "boxer")))),
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
	o	 model
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
func (s stringer) Init() tea.Cmd                           { return nil }
func (s stringer) Update(msg tea.Msg) (tea.Model, tea.Cmd) { return s, nil }
func (s stringer) View() string                            { return s.String() }

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
