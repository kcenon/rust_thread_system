#!/bin/bash

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Default values
CLEAN=false
DEBUG=false
RELEASE=true
EXAMPLES=true
DOCS=false
RUN_TESTS=false
CHECK=false
PACKAGE=""
FEATURES=""

# Function to display usage
show_help() {
    echo "Rust Thread System Build Script"
    echo "Usage: $0 [options]"
    echo ""
    echo "Options:"
    echo "  -h, --help         Show this help message"
    echo "  -c, --clean        Clean build artifacts before building"
    echo "  -d, --debug        Build in debug mode (default is release)"
    echo "  -r, --release      Build in release mode (default)"
    echo "  -e, --examples     Build examples (default)"
    echo "  --no-examples      Don't build examples"
    echo "  --docs             Generate documentation"
    echo "  -t, --test         Run tests"
    echo "  --check            Run cargo check instead of building"
    echo "  -f, --features     Specify features to enable (comma-separated)"
    echo "  -p, --package      Build only the specified package"
    echo ""
    echo "Examples:"
    echo "  $0                  # Build in release mode"
    echo "  $0 -d -t            # Build in debug mode and run tests"
    echo "  $0 -c -r --docs     # Clean, build in release mode, and generate docs"
    echo "  $0 -f \"async,tracing\" # Build with async and tracing features"
    echo ""
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--help)
            show_help
            exit 0
            ;;
        -c|--clean)
            CLEAN=true
            shift
            ;;
        -d|--debug)
            DEBUG=true
            RELEASE=false
            shift
            ;;
        -r|--release)
            RELEASE=true
            DEBUG=false
            shift
            ;;
        -e|--examples)
            EXAMPLES=true
            shift
            ;;
        --no-examples)
            EXAMPLES=false
            shift
            ;;
        --docs)
            DOCS=true
            shift
            ;;
        -t|--test)
            RUN_TESTS=true
            shift
            ;;
        --check)
            CHECK=true
            shift
            ;;
        -f|--features)
            FEATURES="$2"
            shift 2
            ;;
        -p|--package)
            PACKAGE="$2"
            shift 2
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            show_help
            exit 1
            ;;
    esac
done

# Build features string if specified
FEATURES_ARG=""
if [ -n "$FEATURES" ]; then
    FEATURES_ARG="--features $FEATURES"
fi

# Package argument if specified
PACKAGE_ARG=""
if [ -n "$PACKAGE" ]; then
    PACKAGE_ARG="--package $PACKAGE"
fi

# Define build directory
PROJECT_DIR=$(pwd)

# Clean build artifacts if requested
if $CLEAN; then
    echo -e "${YELLOW}Cleaning build artifacts...${NC}"
    cargo clean $PACKAGE_ARG
fi

# Run cargo check
if $CHECK; then
    echo -e "${YELLOW}Checking for compilation errors...${NC}"
    cargo check $PACKAGE_ARG $FEATURES_ARG
    exit $?
fi

# Build in debug mode
if $DEBUG; then
    echo -e "${YELLOW}Building in debug mode...${NC}"
    cargo build $PACKAGE_ARG $FEATURES_ARG
    
    if [ $? -ne 0 ]; then
        echo -e "${RED}Debug build failed!${NC}"
        exit 1
    else
        echo -e "${GREEN}Debug build successful!${NC}"
    fi
fi

# Build in release mode
if $RELEASE; then
    echo -e "${YELLOW}Building in release mode...${NC}"
    cargo build --release $PACKAGE_ARG $FEATURES_ARG
    
    if [ $? -ne 0 ]; then
        echo -e "${RED}Release build failed!${NC}"
        exit 1
    else
        echo -e "${GREEN}Release build successful!${NC}"
    fi
fi

# Build examples
if $EXAMPLES; then
    echo -e "${YELLOW}Building examples...${NC}"
    cargo build --examples $FEATURES_ARG
    
    if [ $? -ne 0 ]; then
        echo -e "${RED}Examples build failed!${NC}"
        exit 1
    else
        echo -e "${GREEN}Examples build successful!${NC}"
    fi
fi

# Run tests
if $RUN_TESTS; then
    echo -e "${YELLOW}Running tests...${NC}"
    cargo test $PACKAGE_ARG $FEATURES_ARG
    
    if [ $? -ne 0 ]; then
        echo -e "${RED}Tests failed!${NC}"
        exit 1
    else
        echo -e "${GREEN}Tests passed!${NC}"
    fi
fi

# Generate documentation
if $DOCS; then
    echo -e "${YELLOW}Generating documentation...${NC}"
    cargo doc --no-deps $FEATURES_ARG
    
    if [ $? -ne 0 ]; then
        echo -e "${RED}Documentation generation failed!${NC}"
        exit 1
    else
        echo -e "${GREEN}Documentation generated successfully!${NC}"
        echo -e "You can view the documentation by opening: ${YELLOW}file://$PROJECT_DIR/target/doc/rust_thread_system/index.html${NC}"
    fi
fi

echo -e "${GREEN}Build completed successfully!${NC}"
exit 0