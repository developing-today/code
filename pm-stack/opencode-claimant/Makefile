# Makefile for OpenCode Claimant/Executor

.PHONY: all install clean test-poll test-claimant shadow live shadow-mode test-requirements test test-harness

all: claimant.py poll.py
	chmod +x claimant.py poll.py
	@echo "Scripts made executable"

install:
	mkdir -p ~/.local/state/opencode-claimant
	@echo "State directory created at ~/.local/state/opencode-claimant"

clean:
	rm -f ~/.local/state/opencode-claimant/*.lock
	@echo "Lock files cleaned"

# All test aliases are offline; none invokes an operator entry point.
test-poll test-claimant test-requirements: test

test:
	python3 -I run_hermetic_tests.py

test-harness:
	python3 -I test_harness_safety.py

shadow:
	@echo "Running in SHADOW mode..."
	python3 claimant.py

live:
	@echo "Setting mode to LIVE..."
	echo "live" > ~/.local/state/opencode-claimant/mode
	python3 claimant.py

shadow-mode:
	@echo "Setting mode to SHADOW..."
	echo "shadow" > ~/.local/state/opencode-claimant/mode
	@echo "Mode set to shadow"
