# Set up local onboarding and librarian assistance

The Linux x86-64 desktop can import the selected model and runtime from local files. The same
**Install or verify local model files** controls appear in **Review local onboarding** and
**Librarian sources → Optional local search assistance**. Setup does not run a model or publish notes.

1. Obtain `Qwen3.5-9B-UD-Q4_K_XL.gguf` from the
   [Unsloth Qwen3.5-9B GGUF repository](https://huggingface.co/unsloth/Qwen3.5-9B-GGUF).
   It is about 5.97 GB and uses Apache-2.0. This build accepts exactly this full SHA-256:
   `6f5d30666c2d8ae16a306e616d95341dcf3cc46810df84d7e6f5a7d1e4c1b293`.
   A newer file with the same name can refuse; the selected digest is fixed in this build.
2. Obtain a matching Linux x86-64 package from
   [llama.cpp release b12110](https://github.com/ggml-org/llama.cpp/releases/tag/b12110):
   `llama-b12110-bin-ubuntu-vulkan-x64.tar.gz` for GPU assistance, or
   `llama-b12110-bin-ubuntu-x64.tar.gz` for CPU. Extract it separately and keep its original
   libraries, relative symlinks and `LICENSE` together. The runtime uses the
   [MIT license](https://github.com/ggml-org/llama.cpp/blob/b12110/LICENSE).
3. In either desktop setup control, select **Qwen3.5-9B model**, enter the absolute path of the
   local GGUF, and choose **Import verified files**. Keep the view open until the receipt appears.
   Akasha copies and fully checks both the input and stored bytes before making the model available.
4. Select **GPU runtime** or **CPU runtime**, enter the absolute path of the extracted folder
   containing `llama-server`, libraries and `LICENSE`, and import it. The archive itself is not an
   accepted input. Akasha copies its pinned executable/library inventory, relative links and license;
   unrelated upstream tools are not installed. Model and runtime are independent installations.
5. After success, select the matching device and run onboarding generation or librarian assistance.
   Use **Inspect model setup** in another already-open panel to refresh its presence display.
   **Verify installed artifact** performs full checks of the selected asset without copying or
   starting a worker. Generation still verifies its complete artifacts before and after completions.

The fixed destination is `~/.local/share/akasha/local-inference/`, under `models/` and
`runtimes/llama-b12110-{vulkan,cpu}/`. No source file is moved or deleted. Import needs space for
the complete new artifact; streaming memory stays bounded. There is no automatic download,
alternative-model selection or destination setting. A successful check establishes the selected
file identities; device compatibility and CPU latency remain separate acceptance limits.

Existing valid assets produce **Already installed and verified**. Existing invalid files or
directories are preserved and refuse; inspect and explicitly move them aside before trying a
replacement. Missing original `LICENSE`, altered libraries, unexpected loader libraries, redirected
links and symlinked source/destination ancestors refuse. Use real paths. Installation directories
must belong to you and disallow group/other writes.

**Cancel setup**, closing the view, or changing projects cancels the owned request and discards
late UI results. A new request waits for native cleanup. A file already committed atomically stays
installed; inspect/verify setup afterward. Each asset commits separately, so a later runtime failure
does not remove the model. The current request deadline is 120 seconds, including copying and
verification; slow storage can refuse. Ordinary failures clean private staging and release the
installation lock. After an abrupt process/OS death, inspect `.setup-lock` in the installation and
`.import-*` remnants in `models/` or `runtimes/` only after confirming no setup is running; automatic
crash-remnant removal and replacement are not implemented.

[Interactive recorded replay](../akasha/Research/evidence/2026-11-10-offline-setup/demo.html) ·
[Real imported-model acceptance](../akasha/Research/local_inference_setup.md) ·
[Onboarding guide](local-onboarding.md) · [Librarian guide](local-librarian.md).
