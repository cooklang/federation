# Mobile UI fixes — design

Date: 2026-09-13

## Problem

Audit of recipes.cooklang.org at a 390px viewport (iPhone width) found:

- **Search form overflows.** Input, language select and Search button sit in one
  non-wrapping flex row. The page becomes 580px wide, the select is clipped and
  the Search button is off-screen.
- **Feeds page.** The "View Recipes" button sits beside the feed title and
  squeezes the title/URL column to ~200px. The metadata row does not wrap. The
  four stat cards stack as full-width blocks.
- **Recipe page.** 48px title inside a card with 32px padding; step text is
  narrowed to ~230px by nested paddings; hero image renders as a 358px square;
  action buttons wrap at arbitrary widths.
- **All pages.** 36px h1 and 32px vertical padding waste the first screen; the
  five-line search-syntax hint pushes results below the fold; card hover
  transforms fire and stick on touch screens.

## Approach

Tailwind responsive utilities in the templates plus a small mobile media block
in `recipe.css`. No new dependencies, no JS changes.

1. **Search form** stacks on `< sm` (`flex-col sm:flex-row`); the select and
   button share a row below the input. The syntax hint moves into a
   `<details>` "Search tips" on small screens and stays a paragraph on `sm+`
   (one Askama macro, rendered twice).
2. **Feeds** use a 2x2 stats grid on phones, the button drops below the feed
   text, metadata wraps.
3. **Recipe page** gets smaller title and card padding on phones, a shorter
   hero image, tighter step gutter and full-width action buttons.
4. **Global**: h1 `text-3xl sm:text-4xl`, page padding `py-6 sm:py-8`,
   wrap-safe pagination, header title `text-xl sm:text-2xl`, 44px hamburger
   target, hover transforms only under `@media (hover: hover)`.
5. **Recipe card partial.** The card markup was copy-pasted in four templates;
   it becomes `_recipe_card.html` with `loading="lazy"` on images.

## Verification

Build CSS with the repo's Tailwind CLI, run the server against the local DB,
mirror each page into a 390px and 320px iframe and check
`documentElement.scrollWidth == innerWidth` on every page.
