#!/usr/bin/env bash
# 第三方核验脚本：核对 GateLang 形式化验证清单是否与链上锚定一致，并本地重跑证明。
#
# 用法：
#   ./verification/verify.sh                 # 核验两份清单
#   ./verification/verify.sh erc20_core      # 只核验一份
#
# 依赖：cast（foundry）、cargo、python3、sha256sum、curl
set -euo pipefail

ANCHOR="${ANCHOR:-0x17dC325d1F8Ff24c8BCF2419b4060C7D062B98f1}"
RPC="${SEPOLIA_RPC:-https://ethereum-sepolia-rpc.publicnode.com}"
REPO_RAW="${REPO_RAW:-https://raw.githubusercontent.com/functioncomplete/gatelang/main}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"

NAMES=("$@")
[ ${#NAMES[@]} -eq 0 ] && NAMES=(erc20_core uniswap_v2_swap)

fail=0
for name in "${NAMES[@]}"; do
  echo "════════ $name ════════"
  manifest="$HERE/$name.verification.json"
  [ -f "$manifest" ] || { echo "  ✗ 缺少清单：$manifest"; fail=1; continue; }

  # 1) 清单哈希 ↔ 链上
  mh="$(cast keccak "$(cat "$manifest")")"
  onchain="$(cast call "$ANCHOR" "isAnchored(bytes32)(bool)" "$mh" --rpc-url "$RPC" 2>/dev/null | tr -d '\r')"
  if [ "$onchain" = "true" ]; then
    echo "  ✓ 清单哈希已锚定：$mh"
  else
    echo "  ✗ 清单哈希未锚定（清单被改过？）：$mh  → $onchain"
    fail=1
    continue
  fi

  # 2) 链上记录字段 ↔ 清单
  idx_plus="$(cast call "$ANCHOR" "indexOf(bytes32)(uint256)" "$mh" --rpc-url "$RPC" 2>/dev/null | tr -d '\r')"
  idx=$((idx_plus - 1))
  rec="$(cast call "$ANCHOR" \
    "recordAt(uint256)((bytes32,bytes32,bytes32,uint32,uint32,uint32,uint32,uint64,address,string))" \
    "$idx" --rpc-url "$RPC" 2>/dev/null | tr -d '\r')"
  want="$(python3 -c "import json,sys;d=json.load(open(sys.argv[1]));s=d['summary'];print(d['sourceSha256'], s['proven']+s['refuted']+s['unknown'], s['proven'], s['refuted'], s['unknown'])" "$manifest")"
  if echo "$rec" | grep -qi "$(echo "$want" | cut -d' ' -f1)"; then
    echo "  ✓ 链上 sourceHash 与清单一致"
  else
    echo "  ✗ 链上 sourceHash 与清单不一致"
    fail=1
  fi

  # 3) 源码哈希 ↔ 清单
  src="$ROOT/examples/$name.gat"
  if [ -f "$src" ]; then
    actual="$(sha256sum "$src" | cut -d' ' -f1)"
    claimed="$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['sourceSha256'])" "$manifest")"
    if [ "$actual" = "$claimed" ]; then
      echo "  ✓ 源码 sha256 一致（$actual）"
    else
      echo "  ✗ 源码已被替换：实际 $actual ≠ 清单 $claimed"
      fail=1
    fi
  fi

  # 4) 本地重跑证明
  bin="$ROOT/target/release/gatelang"
  if [ -x "$bin" ]; then
    rerun="$(mktemp)"
    "$bin" "$src" --prove --json > "$rerun" 2>/dev/null || true
    if python3 -c "import json,sys;json.load(open(sys.argv[1]))" "$rerun" 2>/dev/null; then
      if diff -q <(python3 -m json.tool "$rerun") <(python3 -m json.tool "$manifest") >/dev/null; then
        echo "  ✓ 本地重跑结果与清单逐字节一致"
      else
        echo "  ✗ 本地重跑结果与清单不同（工具版本或源码已变）"
        diff <(python3 -m json.tool "$rerun") <(python3 -m json.tool "$manifest") | head -20 || true
        fail=1
      fi
    else
      echo "  ! 未能重跑（跳过）；请先 cargo build --release"
    fi
    rm -f "$rerun"
  else
    echo "  ! 未构建 gatelang（跳过重跑）；执行 cargo build --release 后重试"
  fi
done

echo
if [ "$fail" -eq 0 ]; then
  echo "全部核验通过 ✓"
else
  echo "存在核验失败 ✗"
fi
exit "$fail"
