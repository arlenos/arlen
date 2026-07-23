import { describe, it, expect } from "vitest";
import { isLocalProvider, providerDisplay } from "./transparency";

describe("isLocalProvider", () => {
  it("recognises a catalog id by its family, not just the bare kind", () => {
    // The real configured provider is a catalog id (`ollama-default`), not the
    // bare kind. An exact-set lookup missed it and the Cost feed called a local
    // model "a cloud service" with a cost - a lie on the honesty surface.
    expect(isLocalProvider("ollama-default")).toBe(true);
    expect(isLocalProvider("ollama")).toBe(true);
    expect(isLocalProvider("ollama-custom")).toBe(true);
    expect(isLocalProvider("llamacpp-local")).toBe(true);
    expect(isLocalProvider("llama.cpp")).toBe(true);
    expect(isLocalProvider("LocalAI")).toBe(true); // case-insensitive
  });

  it("does not mistake a cloud provider for local", () => {
    expect(isLocalProvider("openai")).toBe(false);
    expect(isLocalProvider("anthropic")).toBe(false);
    expect(isLocalProvider("openai-gpt4")).toBe(false);
    expect(isLocalProvider(null)).toBe(false);
    expect(isLocalProvider(undefined)).toBe(false);
  });
});

describe("providerDisplay", () => {
  it("names a known cloud provider by family, not the raw catalog id", () => {
    // The catalog id carries a suffix (`openai-gpt4`); the exact-map miss showed
    // "Openai-gpt4" instead of the brand.
    expect(providerDisplay("openai")).toBe("OpenAI");
    expect(providerDisplay("openai-gpt4")).toBe("OpenAI");
    expect(providerDisplay("anthropic-claude")).toBe("Anthropic");
  });

  it("keeps an unknown provider's full id rather than truncating to the family", () => {
    // A user's custom provider must not become "My" - it keeps its whole name.
    expect(providerDisplay("my-local-llm")).toBe("My-local-llm");
    expect(providerDisplay(null)).toBe(null);
  });
});
