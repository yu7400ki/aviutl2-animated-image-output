import { clsx } from "clsx";
import type { ReactNode } from "react";

export interface StepProps {
  number: number;
  title: string;
  description: string;
  children?: ReactNode;
  className?: string;
}

export function Step({
  number,
  title,
  description,
  children,
  className = "",
}: StepProps) {
  return (
    <li className={clsx("flex items-start", className)}>
      <span className="bg-blue-500 text-white rounded-full w-8 h-8 flex items-center justify-center text-sm font-bold mr-4 mt-0.5 shrink-0">
        {number}
      </span>
      <div>
        <p className="font-semibold text-gray-900 mb-2">{title}</p>
        <p className="text-gray-600">{description}</p>
        {children}
      </div>
    </li>
  );
}
