import { IconCloud } from "@tabler/icons-react";
import { useState } from "react";
import deepseek from "../assets/providers/deepseek.svg?no-inline";
import groq from "../assets/providers/groq.svg?no-inline";
import lmstudio from "../assets/providers/lmstudio.svg?no-inline";
import mistral from "../assets/providers/mistral.png?no-inline";
import ollama from "../assets/providers/ollama.png?no-inline";
import openai from "../assets/providers/openai.png?no-inline";
import openrouterDark from "../assets/providers/openrouter-dark.svg?no-inline";
import openrouterLight from "../assets/providers/openrouter-light.svg?no-inline";
import together from "../assets/providers/together.svg?no-inline";
import {
  providerConnectionBrand,
  providerPresetBrand,
  type ProviderBrand,
  type ProviderConnectionIdentity,
} from "../providerBrand";
import "./provider-icon.css";

const assets: Record<ProviderBrand, { light: string; dark?: string }> = {
  openai: { light: openai },
  openrouter: { light: openrouterLight, dark: openrouterDark },
  groq: { light: groq },
  together: { light: together },
  deepseek: { light: deepseek },
  mistral: { light: mistral },
  ollama: { light: ollama },
  lmstudio: { light: lmstudio },
};

/** Decorative: keep the provider's visible name beside this icon. */
export function ProviderIcon({
  presetId,
  provider,
  size = 20,
}: {
  presetId?: string;
  provider?: ProviderConnectionIdentity | null;
  size?: number;
}) {
  const brand =
    presetId === undefined
      ? providerConnectionBrand(provider)
      : providerPresetBrand(presetId);
  const [failedBrand, setFailedBrand] = useState<ProviderBrand | null>(null);
  const asset = brand && brand !== failedBrand ? assets[brand] : null;
  return (
    <span
      className="provider-icon"
      data-provider-brand={asset ? brand : "custom"}
      aria-hidden="true"
      style={{ width: size, height: size }}
    >
      {asset ? (
        <>
          <img
            className={asset.dark ? "provider-icon-light" : undefined}
            src={asset.light}
            width={size}
            height={size}
            alt=""
            draggable={false}
            onError={() => setFailedBrand(brand)}
          />
          {asset.dark ? (
            <img
              className="provider-icon-dark"
              src={asset.dark}
              width={size}
              height={size}
              alt=""
              draggable={false}
              onError={() => setFailedBrand(brand)}
            />
          ) : null}
        </>
      ) : (
        <IconCloud size={size} />
      )}
    </span>
  );
}
