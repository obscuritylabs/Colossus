import { IconChevronDown, IconPhoto, IconX } from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import type { ImagePreviewLoader, ViewedImage } from "../viewed-images";

function ImagePreview({
  image,
  load,
  onClose,
}: {
  image: ViewedImage;
  load: ImagePreviewLoader;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [source, setSource] = useState("");
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    const element = dialog.current!;
    const opener =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    element.showModal();
    return () => {
      element.close();
      opener?.focus();
    };
  }, []);
  useEffect(() => {
    let current = true;
    setSource("");
    setFailed(false);
    void load(image.artifactId, false).then(
      (url) => {
        if (!current) return;
        if (url.startsWith("data:image/png;base64,")) setSource(url);
        else setFailed(true);
      },
      () => {
        if (current) setFailed(true);
      },
    );
    return () => {
      current = false;
    };
  }, [image.artifactId, load]);
  return (
    <dialog
      ref={dialog}
      className="settings-dialog viewed-image-dialog"
      aria-label={`Image preview: ${image.fileName}`}
      onCancel={onClose}
      onClose={onClose}
    >
      <header>
        <div>
          <strong>{image.fileName}</strong>
          <p>
            {image.widthPixels} × {image.heightPixels} · Preview up to 2048 px
          </p>
        </div>
        <button
          type="button"
          className="button secondary compact"
          aria-label="Close image preview"
          onClick={onClose}
        >
          <IconX size={18} aria-hidden="true" />
        </button>
      </header>
      {source ? (
        <img
          src={source}
          alt={image.fileName}
          onError={() => {
            setSource("");
            setFailed(true);
          }}
        />
      ) : (
        <p role="status">
          {failed ? "Image preview unavailable." : "Loading image…"}
        </p>
      )}
    </dialog>
  );
}

export function ViewedImageGallery({
  images,
  load,
}: {
  images: ViewedImage[];
  load?: ImagePreviewLoader | undefined;
}) {
  const [previews, setPreviews] = useState<Record<string, string | null>>({});
  const [selected, setSelected] = useState<ViewedImage | null>(null);
  useEffect(() => {
    let current = true;
    setPreviews({});
    setSelected(null);
    if (load)
      void (async () => {
        // Read one thumbnail at a time: an original may be 16 MiB. Never download
        // all originals concurrently or load a full preview before the user opens it.
        for (const image of images) {
          if (!current) break;
          let source: string | null = null;
          try {
            const result = await load(image.artifactId, true);
            if (result.startsWith("data:image/png;base64,")) source = result;
          } catch {
            /* Keep the released name and an explicit unavailable state. */
          }
          if (current)
            setPreviews((previous) => ({
              ...previous,
              [image.artifactId]: source,
            }));
        }
      })();
    return () => {
      current = false;
    };
  }, [images, load]);
  return (
    <>
      <ul className="viewed-image-gallery" aria-label="Viewed images">
        {images.map((image, index) => (
          <li key={`${image.artifactId}-${index}`}>
            <button
              type="button"
              className="viewed-image-thumbnail"
              disabled={!load}
              aria-label={`Open image ${index + 1}: ${image.fileName}`}
              onClick={() => setSelected(image)}
            >
              {previews[image.artifactId] ? (
                <img
                  src={previews[image.artifactId]!}
                  alt=""
                  onError={() =>
                    setPreviews((previous) => ({
                      ...previous,
                      [image.artifactId]: null,
                    }))
                  }
                />
              ) : (
                <span className="viewed-image-placeholder">
                  <IconPhoto size={24} aria-hidden="true" />
                  <span>
                    {!load || previews[image.artifactId] === null
                      ? "Preview unavailable"
                      : "Loading…"}
                  </span>
                </span>
              )}
              <span className="viewed-image-name" title={image.fileName}>
                {image.fileName}
              </span>
            </button>
          </li>
        ))}
      </ul>
      {selected && load ? (
        <ImagePreview
          key={selected.artifactId}
          image={selected}
          load={load}
          onClose={() => setSelected(null)}
        />
      ) : null}
    </>
  );
}

export function ViewedImageActivity({
  images,
  createdAt,
  load,
  screenshot = false,
}: {
  images: ViewedImage[];
  createdAt: string;
  load?: ImagePreviewLoader | undefined;
  screenshot?: boolean | undefined;
}) {
  const [open, setOpen] = useState(screenshot);
  return (
    <details
      className="compact-tool-activity activity-tool-thread activity-kind-image activity-state-completed"
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>
        <span className="feed-marker" aria-hidden="true">
          <IconPhoto size={16} stroke={1.7} />
        </span>
        <span className="compact-tool-copy">
          <span className="compact-tool-heading">
            <strong>
              {screenshot
                ? "Browser screenshot"
                : `Viewed ${images.length} ${images.length === 1 ? "image" : "images"}`}
            </strong>
            <time dateTime={createdAt}>
              {new Date(createdAt).toLocaleTimeString([], {
                hour: "numeric",
                minute: "2-digit",
              })}
            </time>
          </span>
        </span>
        <IconChevronDown
          className="compact-tool-chevron"
          size={16}
          stroke={1.8}
          aria-hidden="true"
        />
      </summary>
      {open ? <ViewedImageGallery images={images} load={load} /> : null}
    </details>
  );
}
