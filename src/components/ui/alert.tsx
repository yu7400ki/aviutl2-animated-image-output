import { clsx } from "clsx";
import type { ReactNode } from "react";

export type AlertType = "warning" | "error" | "info" | "success";

export interface AlertProps {
  type: AlertType;
  title: string;
  description?: string;
  children?: ReactNode;
  className?: string;
}

const alertStyles = {
  warning: {
    bg: "bg-yellow-50",
    icon: "bg-yellow-500",
    title: "text-yellow-900",
    text: "text-yellow-800",
  },
  error: {
    bg: "bg-red-50",
    icon: "bg-red-500",
    title: "text-red-900",
    text: "text-red-800",
  },
  info: {
    bg: "bg-blue-50",
    icon: "bg-blue-500",
    title: "text-blue-900",
    text: "text-blue-800",
  },
  success: {
    bg: "bg-green-50",
    icon: "bg-green-500",
    title: "text-green-900",
    text: "text-green-800",
  },
};

export function Alert({
  type,
  title,
  description,
  children,
  className = "",
}: AlertProps) {
  const styles = alertStyles[type];

  return (
    <div className={clsx(styles.bg, "p-6 rounded-lg", className)}>
      <div className="flex items-start">
        <div
          className={clsx(
            "w-6 h-6 rounded-full flex items-center justify-center mr-3 mt-0.5 shrink-0",
            styles.icon,
          )}
        >
          <span className="text-white text-sm font-bold">!</span>
        </div>
        <div>
          <h3 className={clsx("font-semibold mb-2", styles.title)}>{title}</h3>
          {description && <p className={styles.text}>{description}</p>}
          {children}
        </div>
      </div>
    </div>
  );
}
