# cc1 — C89 Compiler + fcc Driver
# ================================

NAME        := cc1
DRIVER      := fcc
CARGO       := cargo
CARGO_FLAGS := --release

# Vendor tools
FT_LEX_DIR  := vendor/ft_lex
FT_YACC_DIR := vendor/ft_yacc

.PHONY: all clean fclean re test lint fmt check vendor

all: vendor $(NAME) $(DRIVER)

$(NAME):
	$(CARGO) build $(CARGO_FLAGS)
	cp target/release/$(NAME) .

$(DRIVER): fcc
	chmod +x fcc

vendor:
	@if [ -f $(FT_LEX_DIR)/Cargo.toml ]; then \
		echo "Building ft_lex..."; \
		cd $(FT_LEX_DIR) && $(CARGO) build --release; \
	fi
	@if [ -f $(FT_YACC_DIR)/Cargo.toml ]; then \
		echo "Building ft_yacc..."; \
		cd $(FT_YACC_DIR) && $(CARGO) build --release; \
	fi

test:
	$(CARGO) test $(CARGO_FLAGS)

lint:
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --check

check: lint fmt test

clean:
	$(CARGO) clean
	rm -f $(NAME)

fclean: clean
	@if [ -f $(FT_LEX_DIR)/Cargo.toml ]; then \
		cd $(FT_LEX_DIR) && $(CARGO) clean; \
	fi
	@if [ -f $(FT_YACC_DIR)/Cargo.toml ]; then \
		cd $(FT_YACC_DIR) && $(CARGO) clean; \
	fi

re: fclean all
