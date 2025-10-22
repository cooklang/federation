# Cooklang Federation Specification v1.0

**Status:** Draft
**Version:** 1.0.0
**Last Updated:** 2025-10-05
**Authors:** Cooklang Federation Working Group

---

## Table of Contents

1. [Introduction](#introduction)
2. [Terminology](#terminology)
3. [Feed Format](#feed-format)
4. [Cooklang Namespace Extension](#cooklang-namespace-extension)
5. [Feed Discovery](#feed-discovery)
6. [Recipe Content Format](#recipe-content-format)
7. [Validation Rules](#validation-rules)
8. [Implementation Guidelines](#implementation-guidelines)
9. [Security Considerations](#security-considerations)
10. [Examples](#examples)

---

## 1. Introduction

The Cooklang Federation Specification defines a standardized format for publishing and discovering Cooklang recipes across the web using syndication feeds. This specification enables:

- Decentralized publishing of recipe collections
- Automated discovery and indexing of recipes
- Interoperability between different Cooklang tools and services
- Preservation of recipe authorship and provenance

### 1.1 Design Goals

- **Simplicity**: Build on existing, well-understood feed formats (Atom/RSS)
- **Compatibility**: Work with standard feed readers and aggregators
- **Extensibility**: Allow for future enhancements without breaking changes
- **Decentralization**: No central authority required for publishing

### 1.2 Scope

This specification covers:
- Feed format and structure
- Cooklang-specific metadata extensions
- Feed discovery mechanisms
- Content encoding requirements

This specification does not cover:
- Recipe parsing and syntax (see [Cooklang Specification](https://cooklang.org/docs/spec/))
- Search and indexing implementation details
- User interface requirements

---

## 2. Terminology

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this document are to be interpreted as described in [RFC 2119](https://www.ietf.org/rfc/rfc2119.txt).

- **Feed**: An Atom or RSS document containing a collection of recipe entries
- **Entry**: A single recipe within a feed
- **Publisher**: An individual or organization publishing a Cooklang feed
- **Indexer**: A service that crawls and indexes Cooklang feeds
- **Consumer**: Any application that reads and processes Cooklang feeds

---

## 3. Feed Format

### 3.1 Supported Formats

Cooklang feeds MUST use one of the following base formats:

- **Atom 1.0** (RFC 4287) - RECOMMENDED
- **RSS 2.0** - SUPPORTED

Atom is RECOMMENDED due to:
- Better standardization (IETF RFC)
- Stricter specification
- Superior internationalization support
- More precise timestamp handling (ISO 8601)

### 3.2 Content Type

Feeds MUST be served with the appropriate MIME type:

- Atom feeds: `application/atom+xml`
- RSS feeds: `application/rss+xml` or `application/xml`

### 3.3 Character Encoding

Feeds MUST use UTF-8 encoding and declare it in the XML declaration:

```xml
<?xml version="1.0" encoding="utf-8"?>
```

---

## 4. Recipe File Links

### 4.1 Link to Recipe File

Each entry MUST include a link to the raw `.cook` file using standard Atom/RSS link elements.

**Atom:**
Use `<link>` with `rel="enclosure"` and `type="text/plain"`:

```xml
<entry>
  <title>Chocolate Chip Cookies</title>
  <link href="https://example.com/recipes/cookies" rel="alternate"/>
  <link href="https://example.com/recipes/cookies.cook" rel="enclosure" type="text/plain"/>
  <!-- ... -->
</entry>
```

**RSS:**
Use `<enclosure>` element:

```xml
<item>
  <title>Chocolate Chip Cookies</title>
  <link>https://example.com/recipes/cookies</link>
  <enclosure url="https://example.com/recipes/cookies.cook" type="text/plain"/>
  <!-- ... -->
</item>
```

### 4.2 Link Requirements

The recipe file link:
- MUST point directly to the raw `.cook` file (not an HTML page)
- MUST be a stable, permanent URL
- SHOULD support HTTP conditional requests (ETag, Last-Modified)
- MUST be served with `Content-Type: text/plain; charset=utf-8`
- MUST use `http://` or `https://` scheme

### 4.3 Multiple Links

Entries MAY include multiple links:
- `rel="alternate"` - Link to HTML page displaying the recipe
- `rel="enclosure"` - Link to raw `.cook` file (REQUIRED)
- `rel="related"` - Link to related resources (images, videos, etc.)

---

## 5. Cooklang Namespace Extension

### 5.1 Namespace Declaration

The Cooklang namespace URI is:

```
https://cooklang.org/feeds/1.0
```

Feeds MUST declare this namespace in the root element:

```xml
<feed xmlns="http://www.w3.org/2005/Atom"
      xmlns:cooklang="https://cooklang.org/feeds/1.0">
```

### 5.2 Extension Elements

All Cooklang-specific elements are OPTIONAL but RECOMMENDED for better discoverability and filtering.

#### 5.2.1 `<cooklang:recipe>`

Container element for Cooklang-specific metadata. This element SHOULD be included in each entry.

**Parent:** `<entry>` (Atom) or `<item>` (RSS)
**Children:** Any Cooklang extension elements
**Occurrence:** 0 or 1

#### 5.2.2 `<cooklang:servings>`

Number of servings the recipe produces.

**Parent:** `<cooklang:recipe>`
**Content:** Positive integer
**Occurrence:** 0 or 1

**Example:**
```xml
<cooklang:servings>4</cooklang:servings>
```

#### 5.2.3 `<cooklang:time>`

Time information for the recipe.

**Parent:** `<cooklang:recipe>`
**Content:** Empty (uses attributes)
**Occurrence:** 0 or 1

**Attributes:**
- `total` - Total time in specified units (OPTIONAL)
- `active` - Active cooking time in specified units (OPTIONAL)
- `units` - Time unit: "minutes", "hours" (REQUIRED if total or active present)

**Example:**
```xml
<cooklang:time total="45" active="20" units="minutes"/>
```

#### 5.2.4 `<cooklang:tags>`

Container for recipe tags/categories.

**Parent:** `<cooklang:recipe>`
**Children:** One or more `<cooklang:tag>` elements
**Occurrence:** 0 or 1

#### 5.2.5 `<cooklang:tag>`

Individual tag/category label.

**Parent:** `<cooklang:tags>`
**Content:** Text string
**Occurrence:** 0 or more

**Example:**
```xml
<cooklang:tags>
  <cooklang:tag>dessert</cooklang:tag>
  <cooklang:tag>cookies</cooklang:tag>
  <cooklang:tag>baking</cooklang:tag>
</cooklang:tags>
```

#### 5.2.6 `<cooklang:difficulty>`

Subjective difficulty rating.

**Parent:** `<cooklang:recipe>`
**Content:** One of: "easy", "medium", "hard"
**Occurrence:** 0 or 1

**Example:**
```xml
<cooklang:difficulty>easy</cooklang:difficulty>
```

#### 5.2.7 `<cooklang:image>`

URL to a recipe image.

**Parent:** `<cooklang:recipe>`
**Content:** Valid HTTP(S) URL
**Occurrence:** 0 or more

**Example:**
```xml
<cooklang:image>https://example.com/images/cookies.jpg</cooklang:image>
```

#### 5.2.8 `<cooklang:nutrition>` (OPTIONAL)

Nutritional information per serving.

**Parent:** `<cooklang:recipe>`
**Content:** Empty (uses attributes)
**Occurrence:** 0 or 1

**Attributes:**
- `calories` - Calories per serving (number)
- `protein` - Protein in grams (number)
- `carbs` - Carbohydrates in grams (number)
- `fat` - Fat in grams (number)
- `fiber` - Fiber in grams (number)

All attributes are OPTIONAL.

**Example:**
```xml
<cooklang:nutrition calories="250" protein="4" carbs="32" fat="12" fiber="2"/>
```

---

## 6. Feed Discovery

### 6.1 Well-Known Locations

Indexers SHOULD check the following locations when discovering feeds:

1. `/.well-known/cooklang-feed` (RECOMMENDED)
2. `/recipes/feed.xml`
3. `/feed.xml`
4. `/atom.xml`
5. `/rss.xml`

### 6.2 HTML Link Discovery

Websites SHOULD include feed discovery links in HTML `<head>`:

```html
<link rel="alternate"
      type="application/atom+xml"
      title="Cooklang Recipes"
      href="/recipes/feed.xml"/>
```

The `rel` attribute MUST be `alternate`.
The `type` attribute MUST match the feed format.

### 6.3 GitHub Repository Discovery

For GitHub repositories:

1. Check for `feed.xml`, `atom.xml`, or `recipes/feed.xml` in repository root
2. If no feed exists, indexers MAY auto-generate a feed from `.cook` files in the repository
3. Use GitHub's raw content URLs for recipe sources

---

## 7. Recipe Content Format

### 7.1 Content Strategy

Cooklang feeds use a **two-tier content model**:

1. **Feed entries** contain metadata and summaries only (lightweight)
2. **Recipe files** (`.cook`) contain the full recipe content (fetched on demand)

This approach:
- Keeps feeds small and fast to parse
- Allows efficient updates (only changed entries need re-fetching)
- Scales to hundreds or thousands of recipes per collection
- Reduces bandwidth for both publishers and indexers

### 7.2 Summary Requirement

Each entry MUST include a `<summary>` (Atom) or `<description>` (RSS).

Summaries SHOULD be concise (1-3 sentences) and MAY include:
- Brief description of the dish
- Key ingredients or flavors
- Cooking method or cuisine type
- Occasion or serving suggestions

**Atom:**
```xml
<summary>Classic chocolate chip cookies with a crispy edge and chewy center. Perfect for dessert or snacking.</summary>
```

**RSS:**
```xml
<description>Classic chocolate chip cookies with a crispy edge and chewy center.</description>
```

### 7.3 Recipe File Format

Recipe files served via enclosure links:
- MUST be valid Cooklang format per the [Cooklang Specification](https://cooklang.org/docs/spec/)
- MUST be UTF-8 encoded plain text
- SHOULD not exceed 100KB in size
- SHOULD include metadata in YAML front matter

**Example `.cook` file:**
```
>> servings: 24
>> time: 45 minutes
>> tags: dessert, cookies, baking

Preheat #oven to 180°C.

Cream @butter{200%g} and @sugar{150%g} together for ~{5%minutes} until fluffy.

Add @eggs{2} and @vanilla extract{1%tsp}, mix well.

...
```

### 7.4 Feed Pagination

For large recipe collections (50+ recipes), publishers SHOULD:

1. Limit feed to 25-50 entries per page
2. Use Atom pagination (RFC 5005):

```xml
<feed xmlns="http://www.w3.org/2005/Atom">
  <!-- feed metadata -->

  <link rel="first" href="https://example.com/recipes/feed.xml?page=1"/>
  <link rel="next" href="https://example.com/recipes/feed.xml?page=2"/>
  <link rel="previous" href="https://example.com/recipes/feed.xml?page=1"/>
  <link rel="last" href="https://example.com/recipes/feed.xml?page=5"/>

  <!-- entries -->
</feed>
```

3. Order entries by most recently updated first
4. Provide an index page listing all available feed pages

---

## 8. Validation Rules

### 8.1 Feed-Level Requirements

- MUST include feed `<title>`
- MUST include at least one `<link>` element (Atom) or `<link>` (RSS)
- MUST include `<updated>` (Atom) or `<lastBuildDate>` (RSS)
- SHOULD include `<author>` information
- MUST include unique `<id>` (Atom)

### 8.2 Entry-Level Requirements

- MUST include entry `<title>`
- MUST include `<link>` with `rel="enclosure"` to `.cook` file (Atom) or `<enclosure>` (RSS)
- MUST include unique `<id>` (Atom) or `<guid>` (RSS)
- MUST include `<updated>` (Atom) or `<pubDate>` (RSS)
- MUST include `<summary>` (Atom) or `<description>` (RSS) with summary text
- SHOULD include `<cooklang:recipe>` with metadata elements

### 8.3 Cooklang Extension Requirements

- If `<cooklang:time>` is present, `units` attribute is REQUIRED
- `<cooklang:difficulty>` MUST be one of: "easy", "medium", "hard"
- `<cooklang:servings>` MUST be a positive integer
- URLs in `<cooklang:image>` MUST be valid HTTP(S) URLs

### 8.4 Content Validation

- Recipe files linked via enclosures MUST be valid Cooklang syntax
- Recipe files MUST be UTF-8 encoded plain text
- Recipe files SHOULD not exceed 100KB
- Summaries SHOULD be 1-3 sentences (approximately 100-500 characters)
- Enclosure URLs MUST use `http://` or `https://` schemes
- Enclosure type MUST be `text/plain`

---

## 9. Implementation Guidelines

### 9.1 Publishers

Publishers implementing Cooklang feeds SHOULD:

1. Update feed `<updated>` timestamp when recipes are added, modified, or removed
2. Update entry `<updated>` timestamp when individual recipes are modified
3. Provide permanent, stable URLs for recipe IDs and `.cook` files
4. Support conditional HTTP requests (ETag, Last-Modified) for both feeds and recipe files
5. Set appropriate cache headers:
   - Feeds: `Cache-Control: max-age=3600` (1 hour)
   - Recipe files: `Cache-Control: max-age=86400` (24 hours)
6. Limit feed to 25-50 entries per page (use pagination for larger collections)
7. Serve `.cook` files with `Content-Type: text/plain; charset=utf-8`
8. Use enclosure URLs that point directly to raw recipe files (not HTML pages)
9. Include descriptive, searchable summaries in each entry
10. Populate as much Cooklang metadata as possible (servings, time, tags, difficulty)

### 9.2 Indexers

Indexers consuming Cooklang feeds SHOULD:

1. Respect `robots.txt` and crawl-delay directives
2. Use conditional HTTP requests (ETag, Last-Modified) to avoid redundant downloads
3. Implement exponential backoff for failed requests
4. Parse both Atom and RSS formats
5. Extract recipe file URLs from enclosure links (`rel="enclosure"` or `<enclosure>`)
6. Fetch recipe content from enclosure URLs:
   - Compare entry `<updated>` timestamp with previously indexed version
   - Only fetch `.cook` file if entry has been updated or is new
   - Cache recipe content locally to avoid redundant fetches
   - Use conditional requests (If-Modified-Since, If-None-Match) when re-fetching
7. Implement a queue system to avoid fetching all recipes simultaneously
8. Validate recipe content (Cooklang syntax) before indexing
9. Store original enclosure URLs and feed URLs for attribution
10. Re-crawl feeds periodically (daily or weekly recommended)
11. Track failed recipe fetches separately from failed feed fetches
12. Respect feed pagination links (rel="next", rel="previous")

### 9.3 Rate Limiting

Indexers MUST:
- Limit requests to 1 per second per domain
- Implement a polite user-agent string identifying the crawler
- Honor HTTP 429 (Too Many Requests) responses

Example user-agent:
```
Cooklang-Indexer/1.0 (+https://federation.example.com/about)
```

---

## 10. Security Considerations

### 10.1 Content Sanitization

Consumers MUST sanitize recipe content before displaying to prevent:
- Cross-Site Scripting (XSS) attacks
- Code injection
- Malformed markup exploitation

### 10.2 URL Validation

Consumers MUST validate and restrict URLs to safe schemes:
- ALLOWED: `http://`, `https://`
- FORBIDDEN: `javascript:`, `data:`, `file://`, etc.

### 10.3 Resource Limits

Consumers SHOULD enforce limits on:
- Maximum feed size (2MB recommended, 5MB maximum)
- Maximum entry count per feed page (50 recommended, 100 maximum)
- Maximum recipe file size (100KB recommended, 1MB maximum)
- Maximum summary length (500 characters recommended)
- Request timeout (30 seconds for feeds, 10 seconds for recipe files)
- Maximum concurrent recipe fetches (10-20 recommended to avoid overwhelming servers)

### 10.4 HTTPS

Publishers SHOULD serve feeds over HTTPS to ensure:
- Content integrity
- Privacy protection
- Authentication of feed source

---

## 11. Examples

### 11.1 Complete Atom Feed Example

```xml
<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"
      xmlns:cooklang="https://cooklang.org/feeds/1.0">

  <title>Jane's Recipe Collection</title>
  <link href="https://example.com/recipes/feed.xml" rel="self"/>
  <link href="https://example.com/recipes/"/>
  <updated>2025-10-05T12:00:00Z</updated>
  <author>
    <name>Jane Doe</name>
    <uri>https://example.com</uri>
    <email>jane@example.com</email>
  </author>
  <id>https://example.com/recipes/feed.xml</id>
  <subtitle>Delicious homemade recipes in Cooklang format</subtitle>

  <entry>
    <title>Perfect Chocolate Chip Cookies</title>
    <link href="https://example.com/recipes/chocolate-chip-cookies" rel="alternate"/>
    <link href="https://example.com/recipes/cookies.cook" rel="enclosure" type="text/plain"/>
    <id>https://example.com/recipes/chocolate-chip-cookies</id>
    <updated>2025-10-01T10:30:00Z</updated>
    <published>2025-09-15T08:00:00Z</published>
    <summary>Classic chocolate chip cookies with a crispy edge and chewy center. These buttery cookies are loaded with chocolate chips and bake up perfectly every time.</summary>

    <cooklang:recipe>
      <cooklang:servings>24</cooklang:servings>
      <cooklang:time total="45" active="20" units="minutes"/>
      <cooklang:tags>
        <cooklang:tag>dessert</cooklang:tag>
        <cooklang:tag>cookies</cooklang:tag>
        <cooklang:tag>baking</cooklang:tag>
      </cooklang:tags>
      <cooklang:difficulty>easy</cooklang:difficulty>
      <cooklang:image>https://example.com/images/cookies.jpg</cooklang:image>
      <cooklang:nutrition calories="150" protein="2" carbs="20" fat="7" fiber="1"/>
    </cooklang:recipe>
  </entry>

  <entry>
    <title>Vegetarian Pad Thai</title>
    <link href="https://example.com/recipes/pad-thai" rel="alternate"/>
    <link href="https://example.com/recipes/pad-thai.cook" rel="enclosure" type="text/plain"/>
    <id>https://example.com/recipes/pad-thai</id>
    <updated>2025-09-28T14:20:00Z</updated>
    <published>2025-09-28T14:20:00Z</published>
    <summary>Quick and easy vegetarian Pad Thai with rice noodles, tofu, and a tangy tamarind sauce.</summary>

    <cooklang:recipe>
      <cooklang:servings>2</cooklang:servings>
      <cooklang:time total="30" active="25" units="minutes"/>
      <cooklang:tags>
        <cooklang:tag>thai</cooklang:tag>
        <cooklang:tag>noodles</cooklang:tag>
        <cooklang:tag>vegetarian</cooklang:tag>
      </cooklang:tags>
      <cooklang:difficulty>medium</cooklang:difficulty>
    </cooklang:recipe>
  </entry>

</feed>
```

### 11.2 RSS 2.0 Feed Example

```xml
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:cooklang="https://cooklang.org/feeds/1.0">
  <channel>
    <title>Jane's Recipe Collection</title>
    <link>https://example.com/recipes/</link>
    <description>Delicious homemade recipes in Cooklang format</description>
    <language>en-us</language>
    <lastBuildDate>Sat, 05 Oct 2025 12:00:00 GMT</lastBuildDate>

    <item>
      <title>Perfect Chocolate Chip Cookies</title>
      <link>https://example.com/recipes/chocolate-chip-cookies</link>
      <enclosure url="https://example.com/recipes/cookies.cook" type="text/plain"/>
      <guid isPermaLink="true">https://example.com/recipes/chocolate-chip-cookies</guid>
      <pubDate>Sun, 15 Sep 2025 08:00:00 GMT</pubDate>
      <description>Classic chocolate chip cookies with a crispy edge and chewy center. These buttery cookies are loaded with chocolate chips and bake up perfectly every time.</description>

      <cooklang:recipe>
        <cooklang:servings>24</cooklang:servings>
        <cooklang:time total="45" active="20" units="minutes"/>
        <cooklang:tags>
          <cooklang:tag>dessert</cooklang:tag>
          <cooklang:tag>cookies</cooklang:tag>
          <cooklang:tag>baking</cooklang:tag>
        </cooklang:tags>
        <cooklang:difficulty>easy</cooklang:difficulty>
        <cooklang:image>https://example.com/images/cookies.jpg</cooklang:image>
      </cooklang:recipe>
    </item>

  </channel>
</rss>
```

### 11.3 Minimal Valid Feed

```xml
<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>My Recipes</title>
  <link href="https://example.com/feed.xml" rel="self"/>
  <updated>2025-10-05T12:00:00Z</updated>
  <id>https://example.com/feed.xml</id>

  <entry>
    <title>Simple Pasta</title>
    <link href="https://example.com/pasta" rel="alternate"/>
    <link href="https://example.com/pasta.cook" rel="enclosure" type="text/plain"/>
    <id>https://example.com/pasta</id>
    <updated>2025-10-05T12:00:00Z</updated>
    <summary>Quick and easy pasta recipe for a weeknight dinner.</summary>
  </entry>
</feed>
```

### 11.4 Example Recipe File (`.cook`)

The file served at `https://example.com/recipes/cookies.cook`:

```
>> servings: 24
>> time: 45 minutes
>> tags: dessert, cookies, baking

Preheat #oven to 180°C.

Cream @butter{200%g} and @sugar{150%g} together for ~{5%minutes} until fluffy.

Add @eggs{2} and @vanilla extract{1%tsp}, mix well.

In a separate bowl, combine @flour{300%g}, @baking soda{1%tsp}, and @salt{1/2%tsp}.

Gradually fold dry ingredients into wet mixture.

Stir in @chocolate chips{200%g}.

Drop spoonfuls onto greased #baking sheet.

Bake for ~{12-15%minutes} until golden brown.

Cool on #wire rack before serving.
```

### 11.5 Paginated Feed Example

```xml
<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Large Recipe Collection</title>
  <link href="https://example.com/feed.xml?page=2" rel="self"/>
  <link href="https://example.com/recipes/"/>
  <updated>2025-10-05T12:00:00Z</updated>
  <id>https://example.com/feed.xml</id>

  <!-- Pagination links -->
  <link rel="first" href="https://example.com/feed.xml?page=1"/>
  <link rel="previous" href="https://example.com/feed.xml?page=1"/>
  <link rel="next" href="https://example.com/feed.xml?page=3"/>
  <link rel="last" href="https://example.com/feed.xml?page=5"/>

  <!-- 25-50 entries on this page -->
  <entry>
    <title>Recipe Title</title>
    <link href="https://example.com/recipe" rel="alternate"/>
    <link href="https://example.com/recipe.cook" rel="enclosure" type="text/plain"/>
    <id>https://example.com/recipe</id>
    <updated>2025-10-05T10:00:00Z</updated>
    <summary>Brief recipe description.</summary>
  </entry>
  <!-- ... more entries ... -->
</feed>
```

---

## Appendix A: Version History

- **1.0.0** (2025-10-05): Initial specification

## Appendix B: References

- [Cooklang Specification](https://cooklang.org/docs/spec/)
- [RFC 4287: The Atom Syndication Format](https://www.ietf.org/rfc/rfc4287.txt)
- [RSS 2.0 Specification](https://www.rssboard.org/rss-specification)
- [RFC 2119: Key words for use in RFCs](https://www.ietf.org/rfc/rfc2119.txt)
- [RFC 3339: Date and Time on the Internet](https://www.ietf.org/rfc/rfc3339.txt)

## Appendix C: License

This specification is released under [CC0 1.0 Universal (Public Domain)](https://creativecommons.org/publicdomain/zero/1.0/).

---

**End of Specification**
