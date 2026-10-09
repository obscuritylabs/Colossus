import {
  useId,
  useRef,
  type FormEvent,
  type HTMLAttributes,
  type ReactNode,
  type RefObject,
  type TextareaHTMLAttributes,
} from "react";
import { IconLoader2, IconPlayerStop, IconSend2 } from "@tabler/icons-react";
import { ComposerInput, isComposerSendKey } from "./Composer.js";
import { Button } from "./Controls.js";

export function ConversationComposerFrame({
  children,
  header,
  footer,
  queue,
  className = "",
  ref,
  ...props
}: Omit<HTMLAttributes<HTMLFormElement>, "onSubmit"> & {
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  header?: ReactNode;
  footer?: ReactNode;
  queue?: ReactNode;
  ref?: RefObject<HTMLFormElement | null>;
}) {
  return (
    <>
      <form
        {...props}
        ref={ref}
        className={`shared-conversation-composer ${className}`}
      >
        {header ? <div className="shared-composer-header">{header}</div> : null}
        <div className="shared-composer-body">{children}</div>
        {footer ? <div className="shared-composer-footer">{footer}</div> : null}
      </form>
      {queue}
    </>
  );
}
export function ConversationComposer({
  value,
  onChange,
  onSubmit,
  disabled = false,
  busy = false,
  label = "Message Colossus",
  placeholder = "What would you like to work on?",
  action = "Send",
  textareaRef,
  formRef,
  controls,
  context,
  queue,
  notice,
  children,
  onStop,
  stopping = false,
  shortcut = "modEnter",
  textareaProps,
  className = "",
}: {
  value: string;
  onChange: (value: string) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  disabled?: boolean;
  busy?: boolean;
  label?: string;
  placeholder?: string;
  action?: string;
  textareaRef?: RefObject<HTMLTextAreaElement | null>;
  formRef?: RefObject<HTMLFormElement | null>;
  controls?: ReactNode;
  context?: ReactNode;
  queue?: ReactNode;
  notice?: ReactNode;
  children?: ReactNode;
  onStop?: () => void;
  stopping?: boolean;
  shortcut?: "enter" | "modEnter";
  textareaProps?: Omit<
    TextareaHTMLAttributes<HTMLTextAreaElement>,
    "value" | "onChange" | "disabled" | "placeholder" | "ref"
  >;
  className?: string;
}) {
  const id = useId(),
    localForm = useRef<HTMLFormElement>(null);
  const targetForm = formRef ?? localForm;
  const inputId = textareaProps?.id ?? id;
  return (
    <div className={`shared-composer-dock ${className}`}>
      <ConversationComposerFrame
        ref={targetForm}
        aria-label={label}
        onSubmit={onSubmit}
        header={context}
        queue={queue}
        footer={
          <>
            <div className="shared-composer-controls">{controls}</div>
            <div className="shared-composer-run-actions">
              {busy && onStop ? (
                <Button
                  variant="tertiary"
                  aria-label="Stop active run"
                  disabled={stopping}
                  onClick={onStop}
                >
                  <IconPlayerStop size={16} aria-hidden="true" />
                  {stopping ? "Stopping…" : "Stop"}
                </Button>
              ) : null}
              <Button
                type="submit"
                variant="primary"
                className="shared-composer-send"
                disabled={disabled || busy || !value.trim()}
                aria-label={action}
              >
                {busy ? (
                  <IconLoader2
                    size={17}
                    className="shared-spin"
                    aria-hidden="true"
                  />
                ) : (
                  <IconSend2 size={17} aria-hidden="true" />
                )}
                {action}
              </Button>
            </div>
          </>
        }
      >
        <label className="shared-sr-only" htmlFor={inputId}>
          {label}
        </label>
        <ComposerInput
          {...textareaProps}
          id={inputId}
          {...(textareaRef ? { ref: textareaRef } : {})}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          disabled={disabled}
          placeholder={placeholder}
          onKeyDown={(event) => {
            textareaProps?.onKeyDown?.(event);
            if (
              !event.defaultPrevented &&
              isComposerSendKey(event.nativeEvent, shortcut)
            ) {
              event.preventDefault();
              if (!disabled && !busy && value.trim())
                targetForm.current?.requestSubmit();
            }
          }}
        />
        {children}
      </ConversationComposerFrame>
      {notice ? (
        <div className="shared-composer-notice" role="status">
          {notice}
        </div>
      ) : null}
    </div>
  );
}
