ALTER TABLE control_plane_settings
    DROP CONSTRAINT classification_colors_valid,
    DROP COLUMN classification_background,
    DROP COLUMN classification_color_text;
