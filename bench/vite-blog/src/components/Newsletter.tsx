import { useState, type FormEvent } from "react";

export function NewsletterSignup() {
  const [email, setEmail] = useState("");
  const [sent, setSent] = useState(false);

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    setSent(true);
  }

  return (
    <form className="nl-form" id="newsletter" onSubmit={onSubmit}>
      <input
        id="nl-email"
        className="nl-input"
        type="email"
        name="email"
        placeholder="you@example.com"
        value={email}
        onChange={(event) => setEmail(event.target.value)}
      />
      <button className="nl-btn" type="submit" id="nl-submit">
        Subscribe
      </button>
      <p className="nl-status" id="nl-status">
        {sent && email
          ? `Thanks — we will not actually email ${email}.`
          : "No tracking pixels. This form stays on the page."}
      </p>
    </form>
  );
}
