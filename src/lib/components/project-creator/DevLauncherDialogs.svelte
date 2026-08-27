<script lang="ts">
  import Modal from "$lib/components/ui/Modal.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import { i18n } from "$lib/core/i18n.svelte";
  import type { TranslationKey } from "$lib/core/i18n.svelte";

  let {
    created,
    confirmCancel,
    onopen,
    onclose,
    oncancel,
    onconfirmcancel,
    ondismisscancel,
  }: {
    created: boolean;
    confirmCancel: boolean;
    onopen: () => void;
    onclose: () => void;
    oncancel: () => void;
    onconfirmcancel: () => void;
    ondismisscancel: () => void;
  } = $props();
</script>

<!-- DevLauncher integration: profile created dialog -->
<Modal
  open={created}
  onclose={onclose}
  title={i18n.t("create.devl_dialog_title") as TranslationKey}
  description={i18n.t("create.devl_dialog_desc") as TranslationKey}
  size="md"
  closeOnBackdrop={false}
  closeOnEscape={false}
>
  {#snippet children()}
    <p style="font-size: 0.9rem; color: var(--sp-text-2); margin: 0 0 0.5rem;">
      {i18n.t("create.profile_added_to_devlauncher") as TranslationKey}
    </p>
    <p style="font-size: 0.85rem; color: var(--sp-text-3); margin: 0;">
      {i18n.t("create.devl_dialog_hint") as TranslationKey}
    </p>
  {/snippet}
  {#snippet footer()}
    <Button variant="subtle" size="sm" onclick={onclose}>
      {i18n.t("create.devl_ok") as TranslationKey}
    </Button>
    <Button variant="danger" size="sm" onclick={oncancel}>
      {i18n.t("create.devl_cancel") as TranslationKey}
    </Button>
    <Button variant="primary" size="sm" onclick={onopen}>
      {i18n.t("create.open_devlauncher") as TranslationKey}
    </Button>
  {/snippet}
</Modal>

<!-- DevLauncher integration: cancel profile confirmation -->
<Modal
  open={confirmCancel}
  onclose={ondismisscancel}
  title={i18n.t("create.devl_cancel_title") as TranslationKey}
  description={i18n.t("create.devl_cancel_desc") as TranslationKey}
  size="sm"
  closeOnBackdrop={false}
  closeOnEscape={false}
>
  {#snippet children()}
    <p style="font-size: 0.9rem; color: var(--sp-text-2); margin: 0;">{i18n.t("create.cancel_profile_confirm") as TranslationKey}</p>
  {/snippet}
  {#snippet footer()}
    <Button variant="subtle" size="sm" onclick={ondismisscancel}>
      {i18n.t("create.devl_keep") as TranslationKey}
    </Button>
    <Button variant="danger" size="sm" onclick={onconfirmcancel}>
      {i18n.t("create.devl_delete") as TranslationKey}
    </Button>
  {/snippet}
</Modal>