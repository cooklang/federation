-- Recipe locale: BCP-47-style code (e.g. 'en', 'de', 'en-US').
-- locale_source records whether the author declared it or we detected it.
ALTER TABLE recipes ADD COLUMN locale TEXT;
ALTER TABLE recipes ADD COLUMN locale_source TEXT;

CREATE INDEX IF NOT EXISTS idx_recipes_locale ON recipes(locale);
