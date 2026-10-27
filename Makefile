PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
CARGO  ?= cargo

.PHONY: all build run test install uninstall clean

all: build

build:
	$(CARGO) build --release --locked

run:
	$(CARGO) run --release --locked

test:
	$(CARGO) test --locked

install: build
	install -d "$(BINDIR)"
	install -m 755 target/release/cadence "$(BINDIR)/cadence"
	@echo "Installed cadence to $(BINDIR)/cadence"
	@case ":$$PATH:" in \
		*":$(BINDIR):"*) ;; \
		*) echo "Note: Add $(BINDIR) to your PATH: export PATH=\"$(BINDIR):\$$PATH\"" ;; \
	esac

uninstall:
	rm -f "$(BINDIR)/cadence"
	@echo "Removed $(BINDIR)/cadence"

clean:
	$(CARGO) clean
