/** The prompt a review session starts with when Settings leaves it blank; mirrors `reviews::DEFAULT_REVIEW_PROMPT`. */
export const DEFAULT_REVIEW_PROMPT =
  'Review pull request #{number} of {repo} ({url}). Make use of code review skills, and at the end give at most 3 options: Approve; Ask <questions here>; Request changes <changes here>. Mark the recommended option "(Recommended)". Show Ask and Request changes only when they are needed.';
