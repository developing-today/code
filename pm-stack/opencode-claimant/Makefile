# Makefile for OpenCode Claimant/Executor

.PHONY: all install clean test-poll test-claimant shadow live shadow-mode test-requirements

all: claimant.py poll.py
	chmod +x claimant.py poll.py
	@echo "Scripts made executable"

install:
	mkdir -p ~/.local/state/opencode-claimant
	@echo "State directory created at ~/.local/state/opencode-claimant"

clean:
	rm -f ~/.local/state/opencode-claimant/*.lock
	@echo "Lock files cleaned"

test-poll:
	@echo "Testing poll.py functions..."
	python3 poll.py --candidates
	python3 poll.py --cap-only
	python3 poll.py --stuck-bound

test-claimant:
	@echo "Testing claimant execution..."
	python3 claimant.py --dry-run

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

test-requirements:
	@echo "Running requirements test suite..."
	python3 -c "import sys; sys.path.insert(0, '.'); from claimant import QUEUES, poll; print('✅ Constants verified'); print(f'  QUEUES: {len(QUEUES)} queues'); print(f'  CONCURRENCY_CAP: {poll.CAP}')"
