(function () {
  var root = document.documentElement;
  var stored = null;
  try {
    stored = localStorage.getItem("bench-theme");
  } catch (err) {
    stored = null;
  }
  if (stored === "dark" || stored === "light") {
    root.setAttribute("data-theme", stored);
  }
  document.querySelectorAll("[data-island='theme']").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var next = root.getAttribute("data-theme") === "dark" ? "light" : "dark";
      root.setAttribute("data-theme", next);
      btn.textContent = next === "dark" ? "Light" : "Dark";
      try {
        localStorage.setItem("bench-theme", next);
      } catch (err) {}
    });
  });
  document.querySelectorAll("[data-island='newsletter']").forEach(function (form) {
    form.addEventListener("submit", function (event) {
      event.preventDefault();
      var status = form.querySelector("[data-nl-status]");
      var input = form.querySelector("input[type='email']");
      if (status) {
        status.textContent = input && input.value ? "Thanks — we will not actually email " + input.value + "." : "Add an email first.";
      }
    });
  });
})();
