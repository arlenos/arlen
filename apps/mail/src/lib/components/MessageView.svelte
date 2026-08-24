<script lang="ts">
  /// The reading surface for one message: the head, then the shared
  /// trust-and-content half (MessageBody). A conversation renders the same
  /// body per block, so a message says the same things wherever it is read.
  import type { Message } from "$lib/stores/mailbox";
  import MessageHeader from "./MessageHeader.svelte";
  import MessageBody from "./MessageBody.svelte";

  let { message }: { message: Message } = $props();
</script>

<article class="message">
  <MessageHeader {message} />
  <MessageBody {message} />
</article>

<style>
  .message {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    width: 100%;
    max-width: 46rem;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 2rem;
  }
</style>
