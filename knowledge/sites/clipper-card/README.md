# Clipper Card

## Login Form Selectors

These selectors were validated against the current Clipper Card login flow during scraper development.

- Email: `input#username[type="email"][name="username"]`
- Password: `input#password[type="password"][name="password"]`
- Submit: `button[type="submit"]`
- CSRF token (hidden): `input[name="_csrf"]`
- Error container: `#form-feedback-container` (contains class `d-none` when hidden)

## Suggested checks in a scraper

- Wait for email field: `await page.waitForSelector('input#username[type="email"][name="username"]')`
- Fill credentials with secret names where possible.
- Click submit and wait for either:
    - URL transition, or
    - visible error state in `#form-feedback-container`.
