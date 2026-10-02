# 形式化验证结果锚定（Sepolia）

本目录存放 **GateLang 形式化验证的可复现清单**，其摘要哈希已锚定到以太坊 Sepolia 测试网。
第三方无需信任任何一方，即可独立核验「某份验证声明确实成立」。

---

## 1. 设计：链上锚定 + 链下全文

形式化验证的完整证明（义务、CNF 规模、冲突数、反例）体积大且可重建，全部上链既不经济也无必要。
因此采用**承诺式锚定**：

| 层 | 内容 | 位置 |
|---|---|---|
| 链上 | `manifestHash`（清单 JSON 的 keccak256）、`sourceHash`（.gat 源码的 sha256）、义务计数、URI | `FCTVerificationAnchor` 合约 |
| 链下 | 完整验证清单 JSON | 本目录（GitHub） |
| 工具 | 可重跑证明的编译器 | `gatelang --prove --json` |

链上只存**可校验的承诺**；只要链下清单被篡改，哈希立刻对不上。

---

## 2. 链上合约

| 项 | 值 |
|---|---|
| 合约 | `FCTVerificationAnchor` |
| 地址 | `0x17dC325d1F8Ff24c8BCF2419b4060C7D062B98f1` |
| 网络 | Sepolia（chainId `11155111`） |
| 浏览器 | https://sepolia.etherscan.io/address/0x17dC325d1F8Ff24c8BCF2419b4060C7D062B98f1 |

**已锚定记录（`count() = 2`）**

| # | 被验证对象 | sourceHash (sha256) | obligations | proven | refuted | unknown |
|---|---|---|---|---|---|---|
| 0 | ERC-20 算术内核 | `0xd09f6fad38e8042d2204ab78331858750b7621e49cdec4fe1858d76d5f5524ff` | 8 | 6 | 2 | 0 |
| 1 | Uniswap V2 `_swap` | `0x5b543a2ff3f62e17d90a4c0151bb409f6382a410a7d67cf2b560a2441ff8b63d` | 3 | 3 | 0 | 0 |

> **关于 refuted（被驳倒）**：ERC-20 清单中的 2 条反例来自**故意植入 bug 的对照组**
> （`ERC20SubBuggy` / `ERC20TransferBuggy`），用于证明证明器**确实能发现错误**——
> 发现不了 bug 的证明器，其「已证明」结论没有意义。

---

## 3. 第三方核验流程

```bash
# 1) 取回清单
curl -sO https://raw.githubusercontent.com/functioncomplete/gatelang/main/verification/erc20_core.verification.json

# 2) 重算哈希，与链上比对
MANIFEST_HASH=$(cast keccak "$(cat erc20_core.verification.json)")
cast call 0x17dC325d1F8Ff24c8BCF2419b4060C7D062B98f1 \
  "isAnchored(bytes32)(bool)" "$MANIFEST_HASH" \
  --rpc-url https://ethereum-sepolia-rpc.publicnode.com
# → true 即清单未被篡改

# 3) 重跑证明，确认清单内容可复现
git clone https://github.com/functioncomplete/gatelang && cd gatelang
cargo build --release
./target/release/gatelang examples/erc20_core.gat --prove --json > rerun.json
diff <(python3 -m json.tool rerun.json) <(python3 -m json.tool ../erc20_core.verification.json)
# → 无差异即验证结果可复现

# 4) 比对被验证源码
python3 -c "import json;print(json.load(open('erc20_core.verification.json'))['sourceSha256'])"
sha256sum examples/erc20_core.gat
# → 两值一致即被验证对象未被替换
```

一键脚本见 [`verify.sh`](./verify.sh)。

### 清单镜像

`raw.githubusercontent.com` 偶有不稳定，可用以下任一镜像取回同一份清单（内容一致）：

```bash
# 官方 raw
curl -sO https://raw.githubusercontent.com/functioncomplete/gatelang/main/verification/erc20_core.verification.json
# jsDelivr CDN
curl -sO https://cdn.jsdelivr.net/gh/functioncomplete/gatelang@main/verification/erc20_core.verification.json
# GitHub API（base64 内嵌）
curl -s https://api.github.com/repos/functioncomplete/gatelang/contents/verification/erc20_core.verification.json?ref=main \
  | python3 -c "import json,sys,base64;print(base64.b64decode(json.load(sys.stdin)['content']).decode())"
```

无论从哪个镜像取，**清单哈希都必须等于链上锚定值**——镜像只影响取回方式，不影响可信性。

---

## 4. 清单格式（`fct-verification/1.0`）

```jsonc
{
  "format": "fct-verification/1.0",
  "kind": "gatelang-verification",
  "tool": "gatelang",
  "toolVersion": "v2.2",
  "method": "CDCL SAT + Tseitin; UNSAT means the property holds for all inputs (not sampling)",
  "sourceFile": "examples/erc20_core.gat",
  "sourceSha256": "d09f…24ff",
  "summary": { "circuits": 5, "proven": 6, "refuted": 2, "unknown": 0, "errors": 0 },
  "reports": [
    {
      "circuit": "FCT.erc20.ERC20SafeSub",
      "preUnsatisfiable": false,
      "obligations": [
        { "kind": "postcondition", "statement": "ok==(a>=b)",
          "verdict": "proven", "cnfVars": 4611, "cnfClauses": 12263, "conflicts": 94 }
      ]
    }
  ]
}
```

**判定语义**（这是「证明」而非「测试」的关键）：

| verdict | 含义 |
|---|---|
| `proven` | CNF 为 **UNSAT** —— 不存在任何输入使性质被违反，是**全称命题** |
| `refuted` | SAT —— 附具体反例输入向量 |
| `unknown` | 触及求解器资源上限，**未下结论**（不谎报） |

---

## 5. 重新生成

```bash
cargo build --release
for f in erc20_core uniswap_v2_swap; do
  ./target/release/gatelang examples/$f.gat --prove --json > verification/$f.verification.json
done
```

清单是**确定性**输出：同一源码 + 同一工具版本 → 逐字节相同的 JSON → 相同的 `manifestHash`。

---

## 6. 重新锚定（新增清单时）

```solidity
anchor(
  manifestHash,  // keccak256(清单 JSON)
  sourceHash,    // sha256(.gat 源码)
  networkHash,   // 逻辑原语 IR 身份（暂无关联填 0）
  obligations, proven, refuted, unknown,
  uri            // 清单全文位置
)
```

`anchor` 无访问控制：任何人都可提交。**可信性不来自提交者身份，而来自「哈希 + 可复现」**——
任何人都能在本地重跑并核对，任何篡改都会在哈希或 diff 处暴露。
