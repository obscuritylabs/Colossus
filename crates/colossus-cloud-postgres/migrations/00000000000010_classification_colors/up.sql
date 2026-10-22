ALTER TABLE control_plane_settings
    ADD COLUMN classification_background TEXT,
    ADD COLUMN classification_color_text TEXT,
    ADD CONSTRAINT classification_colors_valid CHECK (
        (classification_background IS NULL AND classification_color_text IS NULL)
        OR (setting_type = 'display'
            AND classification_background IS NOT NULL
            AND classification_color_text IS NOT NULL
            AND classification_background ~ '^#[0-9A-Fa-f]{6}$'
            AND classification_color_text ~ '^#[0-9A-Fa-f]{6}$')
    );
