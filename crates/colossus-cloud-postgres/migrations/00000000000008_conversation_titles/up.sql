-- NULL retains unknown legacy title provenance and its canonical audit encoding.
ALTER TABLE conversation_threads
    ADD COLUMN title_overridden BOOLEAN,
    ADD COLUMN runtime_title_revision BIGINT CHECK(runtime_title_revision > 0);
