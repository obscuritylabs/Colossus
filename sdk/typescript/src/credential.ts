import { inspect } from "node:util";

export interface MetadataWriter {
  set(name: string, value: string): void;
}

/**
 * An in-memory bearer credential with deliberately redacted representations.
 *
 * The token is never accepted through a descriptor, URL, argv, or environment helper.
 */
export class StaticBearerCredential {
  readonly #token: string;

  public constructor(token: string) {
    if (!/^[\x21-\x7e]{16,761}$/u.test(token)) {
      throw new TypeError(
        "credential must be 16-761 visible ASCII characters",
      );
    }
    this.#token = token;
  }

  public applyTo(metadata: MetadataWriter): void {
    metadata.set("authorization", `Bearer ${this.#token}`);
  }

  public toString(): string {
    return "StaticBearerCredential([REDACTED])";
  }

  public toJSON(): string {
    return "[REDACTED]";
  }

  public [inspect.custom](): string {
    return this.toString();
  }
}
