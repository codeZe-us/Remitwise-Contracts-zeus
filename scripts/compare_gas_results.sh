#!/bin/bash
# Compare gas benchmark results against a baseline
# Usage: ./scripts/compare_gas_results.sh baseline.json current.json [threshold_percent]

set -e

if [ $# -lt 2 ]; then
    echo "Usage: $0 <baseline.json> <current.json> [threshold_percent]"
    echo "Example: $0 baseline.json gas_results.json 10"
    exit 1
fi

BASELINE=$1
CURRENT=$2
THRESHOLD=${3:-10}  # Default 10% increase threshold

if [ ! -f "$BASELINE" ]; then
    echo "Error: Baseline file not found: $BASELINE"
    exit 1
fi

if [ ! -f "$CURRENT" ]; then
    echo "Error: Current results file not found: $CURRENT"
    exit 1
fi

echo "Comparing gas benchmarks (threshold: ${THRESHOLD}% increase)"
echo "Baseline: $BASELINE"
echo "Current:  $CURRENT"
echo ""

# Simple comparison using jq if available
if command -v jq &> /dev/null; then
    REGRESSION=0
    
    # Compare each benchmark
    jq -r '.[] | "\(.contract):\(.method):\(.scenario) \(.cpu) \(.mem)"' "$BASELINE" | while read -r line; do
        KEY=$(echo "$line" | cut -d' ' -f1)
        BASE_CPU=$(echo "$line" | cut -d' ' -f2)
        BASE_MEM=$(echo "$line" | cut -d' ' -f3)
        
        CURR_LINE=$(jq -r ".[] | select(.contract + \":\" + .method + \":\" + .scenario == \"$KEY\") | \"\(.cpu) \(.mem)\"" "$CURRENT")
        
        if [ -n "$CURR_LINE" ]; then
            CURR_CPU=$(echo "$CURR_LINE" | cut -d' ' -f1)
            CURR_MEM=$(echo "$CURR_LINE" | cut -d' ' -f2)
            
            CPU_INCREASE=$(awk "BEGIN {print ($CURR_CPU - $BASE_CPU) / $BASE_CPU * 100}")
            MEM_INCREASE=$(awk "BEGIN {print ($CURR_MEM - $BASE_MEM) / $BASE_MEM * 100}")
            
            printf "%-60s CPU: %+.1f%% MEM: %+.1f%%\n" "$KEY" "$CPU_INCREASE" "$MEM_INCREASE"
            
            if (( $(echo "$CPU_INCREASE > $THRESHOLD" | bc -l) )) || (( $(echo "$MEM_INCREASE > $THRESHOLD" | bc -l) )); then
                echo "  ⚠️  REGRESSION DETECTED"
                REGRESSION=1
            fi
        fi
    done
    
    if [ $REGRESSION -eq 1 ]; then
        echo ""
        echo "❌ Gas regression detected above ${THRESHOLD}% threshold"
        exit 1
    else
        echo ""
        echo "✅ No significant gas regressions"
    fi
else
    echo "⚠️  jq not installed. Install jq for detailed comparison."
    echo "Showing raw results:"
    echo ""
    echo "=== Baseline ==="
    cat "$BASELINE"
    echo ""
    echo "=== Current ==="
    cat "$CURRENT"
fi
